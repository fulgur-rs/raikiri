use super::*;
use raikiri_style::ComputedLength;

fn font(size: f32) -> ChFontKey {
    ChFontKey::new(
        Arc::new(vec![FontFamilyName::named("Test")]),
        ComputedLength(size),
        400.0,
        FontStyle::Normal,
        FontOpticalSizing::Auto,
    )
}

fn assert_bounds<V>(cache: &Measurements<V>) {
    assert_eq!(
        cache.value_count,
        cache.values.values().map(Vec::len).sum::<usize>()
    );
    assert_eq!(
        cache.family_bytes,
        cache
            .values
            .keys()
            .map(|key| family_bytes(&key.0))
            .sum::<usize>()
    );
    assert_eq!(cache.fifo.len(), cache.values.len());
    assert!(cache.value_count <= MAX_VALUES);
    assert!(cache.family_bytes <= MAX_FAMILY_BYTES);
}

#[test]
fn cached_none_and_all_style_optical_pairs_remain_distinct() {
    let mut cache = Measurements::<Option<usize>>::default();
    let mut key = font(16.0);
    for (i, (style, optical)) in [FontStyle::Normal, FontStyle::Italic, FontStyle::Oblique]
        .into_iter()
        .flat_map(|style| {
            [FontOpticalSizing::Auto, FontOpticalSizing::None]
                .into_iter()
                .map(move |optical| (style, optical))
        })
        .enumerate()
    {
        key.style = style;
        key.optical_sizing = optical;
        let expected = if i == 0 { None } else { Some(i) };
        assert_eq!(cache.get_or_measure(&key, || expected), expected);
        assert_eq!(
            cache.get_or_measure(&key, || panic!("a cached value was measured again")),
            expected
        );
    }
    assert_eq!(cache.value_count, 6);
    assert_eq!(cache.fifo.len(), 1);
    assert_bounds(&cache);
}

#[test]
fn exact_size_weight_bits_and_family_kind_are_preserved() {
    let mut cache = Measurements::<usize>::default();
    let mut key = font(16.0);
    for (i, bits) in [
        16.0_f32.to_bits(),
        16.0_f32.to_bits() + 1,
        0,
        (-0.0_f32).to_bits(),
        0x7fc00001,
        0x7fc00002,
    ]
    .into_iter()
    .enumerate()
    {
        key.size = ComputedLength(f32::from_bits(bits));
        assert_eq!(cache.get_or_measure(&key, || i), i);
        assert_eq!(cache.get_or_measure(&key, || panic!("same bits")), i);
    }
    key.weight = f32::from_bits(400.0_f32.to_bits() + 1);
    assert_eq!(cache.get_or_measure(&key, || 42), 42);
    key.family = Arc::new(vec![FontFamilyName::named("serif")]);
    assert_eq!(cache.get_or_measure(&key, || 43), 43);
    key.family = Arc::new(vec![FontFamilyName::generic("serif")]);
    assert_eq!(cache.get_or_measure(&key, || 44), 44);
    assert_eq!(cache.value_count, 9);
    assert_bounds(&cache);
}

#[test]
fn fifo_hit_does_not_promote_and_evicted_key_is_measured_again() {
    let mut cache = Measurements::<usize>::default();
    let first = font(0.0);
    for i in 0..MAX_VALUES {
        cache.get_or_measure(&font(i as f32), || i);
    }
    let retained = cache.family_bytes;
    assert_eq!(cache.get_or_measure(&first, || panic!("hit")), 0);
    assert_eq!(cache.fifo.front().unwrap().1, 0.0_f32.to_bits());
    cache.get_or_measure(&font(MAX_VALUES as f32), || MAX_VALUES);
    assert_eq!(cache.get_or_measure(&first, || 99), 99);
    assert_eq!(cache.family_bytes, retained);
    assert_bounds(&cache);
}

#[test]
fn style_insertion_at_capacity_can_evict_its_own_bucket() {
    let mut cache = Measurements::<usize>::default();
    let mut first = font(0.0);
    for style in [FontStyle::Normal, FontStyle::Italic, FontStyle::Oblique] {
        first.style = style;
        cache.get_or_measure(&first, || 1);
    }
    for i in 1..=MAX_VALUES - 3 {
        cache.get_or_measure(&font(i as f32), || i);
    }
    assert_eq!(cache.value_count, MAX_VALUES);
    first.optical_sizing = FontOpticalSizing::None;
    assert_eq!(cache.get_or_measure(&first, || 2), 2);
    assert_eq!(cache.value_count, MAX_VALUES - 2);
    assert_eq!(
        cache.values[&(first.family.clone(), 0.0_f32.to_bits(), 400.0_f32.to_bits())].len(),
        1
    );
    assert_eq!(cache.get_or_measure(&first, || panic!("retained")), 2);
    assert_bounds(&cache);
}

#[test]
fn family_budget_evicts_whole_buckets_before_the_value_cap() {
    let mut cache = Measurements::<usize>::default();
    for i in 0..8 {
        let mut key = font(i as f32);
        key.family = Arc::new(vec![FontFamilyName::named("x".repeat(100_000))]);
        cache.get_or_measure(&key, || i);
        assert_bounds(&cache);
        assert!(cache.value_count <= 2);
    }
}

#[test]
fn oversized_family_capacity_and_text_are_not_retained() {
    for long_text in [false, true] {
        let mut names = if long_text {
            Vec::new()
        } else {
            Vec::with_capacity(MAX_FAMILY_BYTES / std::mem::size_of::<FontFamilyName>() + 1)
        };
        names.push(FontFamilyName::named(if long_text {
            "x".repeat(MAX_FAMILY_BYTES + 1)
        } else {
            "short".to_owned()
        }));
        let mut key = font(16.0);
        key.family = Arc::new(names);
        let mut cache = Measurements::<usize>::default();
        for i in 0..2 {
            assert_eq!(cache.get_or_measure(&key, || i), i);
        }
        assert!(cache.values.is_empty());
        assert!(cache.fifo.is_empty());
        assert_eq!(Arc::strong_count(&key.family), 1);
        assert_bounds(&cache);
    }
}

#[test]
fn evicting_own_bucket_rechecks_the_incoming_family_capacity() {
    let mut cache = Measurements::<usize>::default();
    let first = font(0.0);
    cache.get_or_measure(&first, || 1);
    for i in 1..MAX_VALUES {
        cache.get_or_measure(&font(i as f32), || i);
    }
    let mut large =
        Vec::with_capacity(MAX_FAMILY_BYTES / std::mem::size_of::<FontFamilyName>() + 1);
    large.push(FontFamilyName::named("Test"));
    let mut key = first;
    key.family = Arc::new(large);
    key.style = FontStyle::Italic;
    assert_eq!(cache.get_or_measure(&key, || 9), 9);
    assert_eq!(Arc::strong_count(&key.family), 1);
    assert_eq!(cache.value_count, MAX_VALUES - 1);
    assert_bounds(&cache);
}

#[test]
fn evicted_family_payload_is_released_from_both_map_and_queue() {
    let mut cache = Measurements::<usize>::default();
    let first = font(0.0);
    let weak = Arc::downgrade(&first.family);
    cache.get_or_measure(&first, || 1);
    drop(first);
    for i in 1..=MAX_VALUES {
        cache.get_or_measure(&font(i as f32), || i);
    }
    assert!(weak.upgrade().is_none());
    assert_bounds(&cache);
}
