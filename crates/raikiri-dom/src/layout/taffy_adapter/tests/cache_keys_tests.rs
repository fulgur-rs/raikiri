//! The cache answers the same lookups the library's cache answers.
//!
//! Both caches see the same stores and the same lookups. The library's cache
//! is the oracle: whether a lookup hits, and what it returns, must not depend
//! on which of the two a lookup went to, including for the inputs whose bits
//! the cache key packs in a special way (`NaN`, `-0.0`, infinities, a missing
//! known dimension).

use super::*;

/// The same operations on the wrapped cache and on the library's.
struct Pair {
    own: LayoutCache,
    taffy: taffy::Cache,
}

impl Pair {
    fn new() -> Self {
        Self {
            own: LayoutCache::new(),
            taffy: taffy::Cache::new(),
        }
    }

    fn store(&mut self, input: &LayoutInput, output: &LayoutOutput) {
        self.own.store(input, output.clone());
        self.taffy
            .store(&(*input).into_taffy(), output.clone().into_taffy());
    }

    /// Look `input` up in both caches and require the same answer.
    fn assert_same_answer(&mut self, input: &LayoutInput, context: &str) {
        let own = self.own.get(input).map(|output| bits_of(&output));
        let taffy = self
            .taffy
            .get(&(*input).into_taffy())
            .map(|output| bits_of(&LayoutOutput::from_taffy(output)));
        assert_eq!(own, taffy, "{context}: lookup of {input:?}");
    }

    fn clear(&mut self) {
        self.own.clear();
        self.taffy.clear();
    }
}

fn input(
    run_mode: RunMode,
    known_dimensions: Size<Option<f32>>,
    available_space: Size<AvailableSpace>,
) -> LayoutInput {
    LayoutInput {
        run_mode,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions,
        known_dimensions_are_definite: Size {
            width: true,
            height: true,
        },
        parent_size: Size::NONE,
        available_space,
        vertical_margins_are_collapsible: Line {
            start: false,
            end: false,
        },
    }
}

/// An output that a measure entry can hold (no margin collapsing metadata) and
/// one that a cache must refuse to keep as a measurement.
fn plain_output(seed: f32) -> LayoutOutput {
    LayoutOutput::from_outer_size(Size {
        width: seed,
        height: seed + 1.0,
    })
}

fn collapsing_output(seed: f32) -> LayoutOutput {
    LayoutOutput {
        top_margin: CollapsibleMarginSet::from_margin(4.0),
        margins_can_collapse_through: true,
        ..plain_output(seed)
    }
}

/// Inputs that differ in one thing each, from the extreme floats to the
/// keywords, so that every pair of them is a pair of keys the cache has to tell
/// apart or to treat as one.
fn probe_inputs() -> Vec<LayoutInput> {
    let known = [
        None,
        Some(f32::NAN),
        Some(-0.0),
        Some(0.0),
        Some(120.0),
        Some(1e7),
        Some(f32::INFINITY),
    ];
    let spaces = [
        AvailableSpace::MinContent,
        AvailableSpace::MaxContent,
        AvailableSpace::Definite(0.0),
        AvailableSpace::Definite(-0.0),
        AvailableSpace::Definite(300.0),
        AvailableSpace::Definite(f32::NAN),
    ];
    let mut inputs = Vec::new();
    for (index, &width) in known.iter().enumerate() {
        for (offset, &space) in spaces.iter().enumerate() {
            inputs.push(input(
                RunMode::ComputeSize,
                Size {
                    width,
                    height: known[(index + offset) % known.len()],
                },
                Size {
                    width: space,
                    height: spaces[(index + 2 * offset) % spaces.len()],
                },
            ));
        }
    }
    inputs
}

#[test]
fn every_probe_hits_or_misses_as_the_library_does() {
    let probes = probe_inputs();
    for (stored_index, stored) in probes.iter().enumerate() {
        let mut caches = Pair::new();
        caches.store(stored, &plain_output(stored_index as f32));
        for probe in &probes {
            caches.assert_same_answer(probe, "after one store");
        }
    }
}

#[test]
fn two_stores_leave_both_caches_answering_alike() {
    let probes = probe_inputs();
    for (first_index, first) in probes.iter().enumerate().step_by(3) {
        for (second_index, second) in probes.iter().enumerate().step_by(5) {
            let mut caches = Pair::new();
            caches.store(first, &plain_output(first_index as f32));
            caches.store(second, &plain_output(1000.0 + second_index as f32));
            for probe in &probes {
                caches.assert_same_answer(probe, "after two stores");
            }
        }
    }
}

#[test]
fn a_full_cache_evicts_the_same_entries() {
    // More measurements than the cache has slots, with lookups in between so
    // that some entries count as recently used.
    let probes = probe_inputs();
    let mut caches = Pair::new();
    for (index, stored) in probes.iter().enumerate() {
        caches.store(stored, &plain_output(index as f32));
        if index % 4 == 0 {
            caches.assert_same_answer(&probes[index / 2], "while filling");
        }
    }
    for probe in &probes {
        caches.assert_same_answer(probe, "after filling");
    }
}

#[test]
fn the_axis_of_an_entry_decides_which_lookups_it_answers() {
    let base = input(
        RunMode::ComputeSize,
        Size {
            width: Some(80.0),
            height: None,
        },
        Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::Definite(50.0),
        },
    );
    let axes = [
        RequestedAxis::Horizontal,
        RequestedAxis::Vertical,
        RequestedAxis::Both,
    ];
    for stored_axis in axes {
        let mut caches = Pair::new();
        caches.store(
            &LayoutInput {
                axis: stored_axis,
                ..base
            },
            &plain_output(1.0),
        );
        for probe_axis in axes {
            caches.assert_same_answer(
                &LayoutInput {
                    axis: probe_axis,
                    ..base
                },
                "an entry of another axis",
            );
        }
    }
}

#[test]
fn parent_size_and_definiteness_are_part_of_the_key() {
    let base = input(
        RunMode::ComputeSize,
        Size {
            width: Some(80.0),
            height: Some(f32::NAN),
        },
        Size {
            width: AvailableSpace::Definite(200.0),
            height: AvailableSpace::MinContent,
        },
    );
    let parents = [
        Size::NONE,
        Size {
            width: Some(500.0),
            height: None,
        },
        Size {
            width: Some(500.0),
            height: Some(-0.0),
        },
        Size {
            width: None,
            height: Some(f32::NAN),
        },
    ];
    let definiteness = [
        Size {
            width: true,
            height: true,
        },
        Size {
            width: false,
            height: true,
        },
        Size {
            width: true,
            height: false,
        },
        Size {
            width: false,
            height: false,
        },
    ];
    for stored_parent in parents {
        for stored_definite in definiteness {
            let mut caches = Pair::new();
            caches.store(
                &LayoutInput {
                    parent_size: stored_parent,
                    known_dimensions_are_definite: stored_definite,
                    ..base
                },
                &plain_output(2.0),
            );
            for probe_parent in parents {
                for probe_definite in definiteness {
                    caches.assert_same_answer(
                        &LayoutInput {
                            parent_size: probe_parent,
                            known_dimensions_are_definite: probe_definite,
                            ..base
                        },
                        "a parent size or definiteness that differs",
                    );
                }
            }
        }
    }
}

#[test]
fn the_sizing_mode_and_margin_collapsing_drop_the_entries_of_another_mode() {
    let base = input(
        RunMode::ComputeSize,
        Size {
            width: Some(10.0),
            height: Some(20.0),
        },
        Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MaxContent,
        },
    );
    let modes = [
        (SizingMode::InherentSize, false, false),
        (SizingMode::ContentSize, false, false),
        (SizingMode::InherentSize, true, false),
        (SizingMode::InherentSize, false, true),
        (SizingMode::ContentSize, true, true),
    ];
    let with_mode = |(sizing_mode, start, end): (SizingMode, bool, bool)| LayoutInput {
        sizing_mode,
        vertical_margins_are_collapsible: Line { start, end },
        ..base
    };
    for stored in modes {
        let mut caches = Pair::new();
        caches.store(&with_mode(stored), &plain_output(3.0));
        for probe in modes {
            caches.assert_same_answer(&with_mode(probe), "a mode that differs");
        }
        // A second store in another mode replaces what the first one stored.
        for other in modes {
            let mut caches = Pair::new();
            caches.store(&with_mode(stored), &plain_output(3.0));
            caches.store(&with_mode(other), &plain_output(4.0));
            for probe in modes {
                caches.assert_same_answer(&with_mode(probe), "after a store in another mode");
            }
        }
    }
}

#[test]
fn final_layouts_are_kept_for_the_input_they_were_computed_for() {
    let probes = probe_inputs();
    for (index, stored) in probes.iter().enumerate().step_by(2) {
        let layout_input = LayoutInput {
            run_mode: RunMode::PerformLayout,
            ..*stored
        };
        let mut caches = Pair::new();
        caches.store(&layout_input, &collapsing_output(index as f32));
        for probe in &probes {
            for run_mode in [RunMode::PerformLayout, RunMode::ComputeSize] {
                caches.assert_same_answer(
                    &LayoutInput { run_mode, ..*probe },
                    "after a store of a final layout",
                );
            }
        }
    }
}

#[test]
fn hidden_layouts_are_never_kept() {
    let probes = probe_inputs();
    let mut caches = Pair::new();
    let hidden = LayoutInput {
        run_mode: RunMode::PerformHiddenLayout,
        ..probes[0]
    };
    caches.store(&hidden, &plain_output(5.0));
    caches.assert_same_answer(&hidden, "hidden layout");
    for probe in &probes {
        caches.assert_same_answer(probe, "after a hidden layout");
    }
}

#[test]
fn a_measurement_with_margin_collapsing_metadata_is_not_kept() {
    let probes = probe_inputs();
    let stored = probes[8];
    let mut caches = Pair::new();
    caches.store(&stored, &collapsing_output(6.0));
    caches.assert_same_answer(&stored, "a measurement that collapses through");
    caches.store(&stored, &plain_output(7.0));
    caches.assert_same_answer(&stored, "the same measurement without metadata");
    assert!(caches.own.get(&stored).is_some());
}

#[test]
fn clear_forgets_every_entry() {
    let probes = probe_inputs();
    let mut caches = Pair::new();
    for (index, stored) in probes.iter().enumerate().take(12) {
        caches.store(stored, &plain_output(index as f32));
    }
    caches.clear();
    for probe in &probes {
        caches.assert_same_answer(probe, "after clear");
        assert!(caches.own.get(probe).is_none());
    }
    // And the cache works as a fresh one after it.
    caches.store(&probes[1], &plain_output(9.0));
    caches.assert_same_answer(&probes[1], "after clear and store");
    assert!(caches.own.get(&probes[1]).is_some());
}

#[test]
fn a_hit_returns_the_stored_output_bit_for_bit() {
    let mut floats = Floats(2);
    let layout_input = LayoutInput {
        run_mode: RunMode::PerformLayout,
        ..layout_input(&mut floats)
    };
    let output = layout_output(&mut floats);
    let mut cache = LayoutCache::new();
    assert!(cache.get(&layout_input).is_none());
    cache.store(&layout_input, output.clone());
    let hit = cache.get(&layout_input).expect("the stored layout");
    assert_eq!(bits_of(&hit), bits_of(&output));
    assert_eq!(format!("{hit:?}"), format!("{output:?}"));
}
