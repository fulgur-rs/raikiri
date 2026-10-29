use super::*;

fn verify(node: &Link, low: Option<&str>, high: Option<&str>) -> u8 {
    let Some(node) = node else {
        return 0;
    };
    assert!(low.is_none_or(|low| low < node.name.as_str()));
    assert!(high.is_none_or(|high| high > node.name.as_str()));
    let left = verify(&node.left, low, Some(&node.name));
    let right = verify(&node.right, Some(&node.name), high);
    assert!((i16::from(left) - i16::from(right)).abs() <= 1);
    assert_eq!(node.height, 1 + left.max(right));
    assert!(
        node.left
            .as_ref()
            .is_none_or(|child| child.generation <= node.generation)
    );
    assert!(
        node.right
            .as_ref()
            .is_none_or(|child| child.generation <= node.generation)
    );
    node.height
}

#[test]
fn all_rotation_shapes_preserve_snapshots_and_flat_map_values() {
    for order in [[3, 2, 1], [1, 2, 3], [3, 1, 2], [1, 3, 2]] {
        let mut index = CustomPropertyIndex::default();
        let mut snapshots = Vec::new();
        let mut flat = HashMap::new();
        for (generation, number) in order.into_iter().enumerate() {
            snapshots.push((index.clone(), flat.clone()));
            let name = SmolStr::from(format!("--{number}"));
            let value = Some(SmolStr::from(number.to_string()));
            flat.insert(name.clone(), value.clone());
            index = CustomPropertyIndex::with_local(
                &index,
                &HashMap::from([(name, value)]),
                generation as u64 + 1,
            )
            .0;
            verify(&index.root, None, None);
            for (name, value) in &flat {
                assert_eq!(index.get::<false>(name), *value);
            }
        }
        for (snapshot, values) in snapshots {
            for number in 0..5 {
                let name = format!("--{number}");
                assert_eq!(
                    snapshot.get::<false>(&name),
                    values.get(name.as_str()).cloned().flatten()
                );
            }
        }
    }
}

fn identities(root: &Link, into: &mut std::collections::HashSet<usize>) {
    if let Some(root) = root {
        into.insert(Arc::as_ptr(root) as usize);
        identities(&root.left, into);
        identities(&root.right, into);
    }
}

fn verify_owned(parent: &CustomPropertyIndex, child: &CustomPropertyIndex, bytes: u64) {
    let mut before = std::collections::HashSet::new();
    let mut after = std::collections::HashSet::new();
    identities(&parent.root, &mut before);
    identities(&child.root, &mut after);
    assert_eq!(bytes, after.difference(&before).count() as u64 * NODE_BYTES);
    verify(&child.root, None, None);
}

#[test]
fn batch_accounting_counts_only_final_new_allocations_even_after_rotations() {
    let empty = CustomPropertyIndex::default();
    let first: HashMap<_, _> = (0..128)
        .map(|i| (format!("--name{i:04}").into(), Some("old".into())))
        .collect();
    let (parent, bytes) = CustomPropertyIndex::with_local(&empty, &first, 1);
    verify_owned(&empty, &parent, bytes);
    for range in [0..128, 64..256, 128..512] {
        let updates: HashMap<_, _> = range
            .map(|i| {
                (
                    format!("--name{i:04}").into(),
                    if i % 3 == 0 { None } else { Some("new".into()) },
                )
            })
            .collect();
        let (child, bytes) = CustomPropertyIndex::with_local(&parent, &updates, 2);
        verify_owned(&parent, &child, bytes);
        for index in 0..512 {
            let name = format!("--name{index:04}");
            let expected = updates
                .get(name.as_str())
                .or_else(|| first.get(name.as_str()))
                .cloned()
                .flatten();
            assert_eq!(child.get::<false>(&name), expected);
            assert_eq!(
                parent.get::<false>(&name),
                first.get(name.as_str()).cloned().flatten()
            );
        }
        // A sibling has the same generation, but never reads this child's nodes.
        let sibling_update = HashMap::from([("--sibling".into(), Some("own".into()))]);
        let (sibling, bytes) = CustomPropertyIndex::with_local(&parent, &sibling_update, 2);
        verify_owned(&parent, &sibling, bytes);
        assert_eq!(child.get::<false>("--sibling"), None);
        let (redeclared, bytes) = CustomPropertyIndex::with_local(&sibling, &sibling_update, 3);
        verify_owned(&sibling, &redeclared, bytes);
        assert_eq!(bytes, 0);
        assert!(Arc::ptr_eq(
            sibling.root.as_ref().unwrap(),
            redeclared.root.as_ref().unwrap()
        ));
    }
}

#[test]
fn ordered_reverse_and_mixed_names_keep_logarithmic_height_and_snapshot_values() {
    for mode in 0..3 {
        let mut index = CustomPropertyIndex::default();
        for i in 0..1024 {
            let key = match mode {
                0 => i,
                1 => 1023 - i,
                _ => (i * 733) % 1024,
            };
            let local = HashMap::from([(
                SmolStr::from(format!("--{key:04}")),
                Some(SmolStr::from(key.to_string())),
            )]);
            let (next, bytes) = CustomPropertyIndex::with_local(&index, &local, i as u64 + 1);
            verify_owned(&index, &next, bytes);
            index = next;
        }
        assert!(height(&index.root) <= 16);
        for key in 0..1024 {
            assert_eq!(
                index.get::<false>(&format!("--{key:04}")),
                Some(key.to_string().into())
            );
        }
        assert_eq!(index.get::<false>("--missing"), None);
    }
}

#[test]
fn heap_text_buffers_are_shared_through_path_copies_and_tombstone_redeclaration() {
    let name = SmolStr::new("--a-name-that-is-too-long-for-inline-storage");
    let value = SmolStr::new("a-value-that-is-also-too-long-for-inline-storage");
    let local = HashMap::from([(name.clone(), Some(value.clone()))]);
    let (parent, bytes) =
        CustomPropertyIndex::with_local(&CustomPropertyIndex::default(), &local, 1);
    assert_eq!(bytes, NODE_BYTES);
    assert_eq!(parent.root.as_ref().unwrap().name.as_ptr(), name.as_ptr());
    assert_eq!(
        parent
            .root
            .as_ref()
            .unwrap()
            .value
            .as_ref()
            .unwrap()
            .as_ptr(),
        value.as_ptr()
    );
    let (hidden, bytes) =
        CustomPropertyIndex::with_local(&parent, &HashMap::from([(name.clone(), None)]), 2);
    verify_owned(&parent, &hidden, bytes);
    assert_eq!(hidden.get::<false>(&name), None);
    assert_eq!(parent.get::<false>(&name), Some(value.clone()));
    let (restored, bytes) = CustomPropertyIndex::with_local(&hidden, &local, 3);
    verify_owned(&hidden, &restored, bytes);
    assert_eq!(restored.get::<false>(&name), Some(value));
}

#[test]
fn retained_byte_budget_is_independent_of_local_hash_iteration_order() {
    let mut expected = None;
    for _ in 0..32 {
        let local: HashMap<_, _> = (0..128)
            .map(|i| {
                (
                    SmolStr::from(format!("--name{i:04}")),
                    Some(SmolStr::from("old")),
                )
            })
            .collect();
        let (parent, _) =
            CustomPropertyIndex::with_local(&CustomPropertyIndex::default(), &local, 1);
        let (_, bytes) = CustomPropertyIndex::with_local(
            &parent,
            &HashMap::from([("--name0064".into(), Some("updated".into()))]),
            2,
        );
        if let Some(expected) = expected {
            assert_eq!(
                bytes, expected,
                "equal binding sets must have the same output-byte budget"
            );
        } else {
            expected = Some(bytes);
        }
    }
}
