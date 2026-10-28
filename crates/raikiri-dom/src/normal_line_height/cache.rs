//! Bounded FIFO buckets for exact font measurements.
//!
//! Each metric has its own 512-value limit and 256 KiB family charge. The
//! charge includes the retained family Vec's capacity and UTF-8 name lengths;
//! shared payloads are conservatively charged per key. Map/queue/style storage
//! is separately bounded by the value limit. This is not an RSS limit.
//! Hits do not update FIFO order or allocate. Cold insertions can evict a hot
//! bucket, after which it is measured again. Oversized families bypass storage.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use raikiri_style::property::{FontOpticalSizing, FontStyle};
use raikiri_style::{ChFontKey, FontFamilyName};

type Key = (Arc<Vec<FontFamilyName>>, u32, u32);
type Styles<V> = Vec<(FontStyle, FontOpticalSizing, V)>;
const MAX_VALUES: usize = 512;
const MAX_FAMILY_BYTES: usize = 256 * 1024;

#[derive(Default)]
pub(super) struct Measurements<V> {
    pub(super) values: HashMap<Key, Styles<V>>,
    fifo: VecDeque<Key>,
    value_count: usize,
    family_bytes: usize,
}

fn family_bytes(family: &Vec<FontFamilyName>) -> usize {
    family.iter().fold(
        family
            .capacity()
            .saturating_mul(std::mem::size_of::<FontFamilyName>()),
        |bytes, name| bytes.saturating_add(name.as_str().len()),
    )
}

impl<V: Copy> Measurements<V> {
    pub(super) fn get_or_measure(&mut self, font: &ChFontKey, measure: impl FnOnce() -> V) -> V {
        let key = (
            font.family.clone(),
            font.size.0.to_bits(),
            font.weight.to_bits(),
        );
        if let Some(styles) = self.values.get(&key)
            && let Some(&(_, _, value)) = styles
                .iter()
                .find(|(style, optical, _)| *style == font.style && *optical == font.optical_sizing)
        {
            return value;
        }
        let value = measure();
        let mut present = self.values.contains_key(&key);
        let mut charge = if present { 0 } else { family_bytes(&key.0) };
        if charge > MAX_FAMILY_BYTES {
            return value;
        }
        while self.value_count == MAX_VALUES
            || self.family_bytes.saturating_add(charge) > MAX_FAMILY_BYTES
        {
            let oldest = self
                .fifo
                .pop_front()
                .expect("retained measurements have FIFO buckets");
            let removed = self
                .values
                .remove(&oldest)
                .expect("FIFO buckets have retained measurements");
            self.value_count -= removed.len();
            self.family_bytes -= family_bytes(&oldest.0);
            // A style miss can evict its own bucket. Reinsert it as a new key
            // with the incoming family's actual capacity charged again.
            if present && oldest.1 == key.1 && oldest.2 == key.2 && oldest.0 == key.0 {
                present = false;
                charge = family_bytes(&key.0);
                if charge > MAX_FAMILY_BYTES {
                    return value;
                }
            }
        }
        if !present {
            self.fifo.push_back(key.clone());
            self.family_bytes += charge;
        }
        self.values
            .entry(key)
            .or_default()
            .push((font.style, font.optical_sizing, value));
        self.value_count += 1;
        value
    }
}

#[cfg(test)]
mod tests;
