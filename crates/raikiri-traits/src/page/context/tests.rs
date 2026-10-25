use super::*;
use crate::dom::NodeId;
use raikiri_style::property::CounterStyle;

// ── CounterStack ─────────────────────────────────────

mod counter_stack_tests {
    use super::*;

    #[test]
    fn fresh_stack_has_no_current_value() {
        let stack = CounterStack::default();
        assert_eq!(stack.current(), None);
        assert!(stack.values().is_empty());
    }

    #[test]
    fn reset_pushes_new_nested_scope_not_replace() {
        let mut stack = CounterStack::default();
        stack.reset(0);
        stack.reset(10);
        assert_eq!(stack.values(), &[0, 10]);
        assert_eq!(stack.current(), Some(10));
    }

    #[test]
    fn increment_on_empty_stack_implicitly_creates_at_delta() {
        let mut stack = CounterStack::default();
        stack.increment(3);
        assert_eq!(stack.current(), Some(3));
    }

    #[test]
    fn increment_mutates_only_innermost_frame() {
        let mut stack = CounterStack::default();
        stack.reset(0);
        stack.reset(10);
        stack.increment(5);
        assert_eq!(stack.values(), &[0, 15]);
    }

    #[test]
    fn increment_saturates_instead_of_panicking_or_wrapping_on_overflow() {
        // Regression check: `counter-reset: c 2147483647;
        // counter-increment: c 1` is spec-legal CSS. Must saturate at
        // i32::MAX, not panic (debug builds) or wrap to i32::MIN
        // (release builds).
        let mut stack = CounterStack::default();
        stack.reset(i32::MAX);
        stack.increment(1);
        assert_eq!(stack.current(), Some(i32::MAX));

        // Symmetric check on the negative side.
        let mut stack = CounterStack::default();
        stack.reset(i32::MIN);
        stack.increment(-1);
        assert_eq!(stack.current(), Some(i32::MIN));
    }

    #[test]
    fn set_on_empty_stack_implicitly_creates_at_value() {
        let mut stack = CounterStack::default();
        stack.set(7);
        assert_eq!(stack.current(), Some(7));
    }

    #[test]
    fn set_mutates_only_innermost_frame() {
        let mut stack = CounterStack::default();
        stack.reset(0);
        stack.reset(10);
        stack.set(99);
        assert_eq!(stack.values(), &[0, 99]);
    }

    #[test]
    fn pop_scope_removes_only_innermost_frame_and_reveals_previous() {
        let mut stack = CounterStack::default();
        stack.reset(1);
        stack.reset(2);
        assert_eq!(stack.pop_scope(), Some(2));
        assert_eq!(stack.current(), Some(1));
        assert_eq!(stack.pop_scope(), Some(1));
        assert_eq!(stack.current(), None);
    }

    #[test]
    fn pop_scope_on_empty_stack_returns_none_without_panicking() {
        let mut stack = CounterStack::default();
        assert_eq!(stack.pop_scope(), None);
    }
}

// ── NamedStringState ─────────────────────────────────

mod named_string_state_tests {
    use super::*;

    #[test]
    fn fresh_state_has_no_snapshots() {
        let state = NamedStringState::default();
        assert_eq!(state.on_page_start(), None);
        assert_eq!(state.on_page_first_use(), None);
        assert_eq!(state.on_page_last_use(), None);
        assert_eq!(state.running(), None);
    }

    #[test]
    fn first_string_set_sets_first_last_and_running_but_not_start() {
        let mut state = NamedStringState::default();
        state.apply_string_set("A".to_owned());
        assert_eq!(state.on_page_first_use(), Some("A"));
        assert_eq!(state.on_page_last_use(), Some("A"));
        assert_eq!(state.running(), Some("A"));
        assert_eq!(state.on_page_start(), None);
    }

    #[test]
    fn second_string_set_same_page_updates_last_not_first() {
        let mut state = NamedStringState::default();
        state.apply_string_set("A".to_owned());
        state.apply_string_set("B".to_owned());
        assert_eq!(
            state.on_page_first_use(),
            Some("A"),
            "first-use must stay pinned to the first assignment on the page"
        );
        assert_eq!(state.on_page_last_use(), Some("B"));
        assert_eq!(state.running(), Some("B"));
    }

    #[test]
    fn begin_page_snapshots_running_into_start_and_clears_first_last() {
        let mut state = NamedStringState::default();
        state.apply_string_set("A".to_owned());
        state.begin_page();
        assert_eq!(state.on_page_start(), Some("A"));
        assert_eq!(state.on_page_first_use(), None);
        assert_eq!(state.on_page_last_use(), None);
        assert_eq!(state.running(), Some("A"));
    }

    #[test]
    fn multi_page_sequence_tracks_snapshot_timing_and_ordering() {
        let mut state = NamedStringState::default();
        state.apply_string_set("A".to_owned());

        state.begin_page();
        assert_eq!(state.on_page_start(), Some("A"));

        state.apply_string_set("B".to_owned());
        state.apply_string_set("C".to_owned());
        assert_eq!(state.on_page_first_use(), Some("B"));
        assert_eq!(state.on_page_last_use(), Some("C"));

        state.begin_page();
        assert_eq!(state.on_page_start(), Some("C"));
        assert_eq!(state.on_page_first_use(), None);
    }
}

// ── resolve_content_source ────────────────────────────

mod resolve_content_source_tests {
    use super::*;

    #[test]
    fn literal_only_resolves_verbatim() {
        let source = ContentSource::new(vec![ContentValueItem::Literal("Ch. ".into())]);
        let counters = HashMap::new();
        assert_eq!(
            resolve_content_source(&source, &counters),
            Some("Ch. ".to_owned())
        );
    }

    #[test]
    fn counter_item_formats_current_value() {
        let mut counters = HashMap::new();
        let mut stack = CounterStack::default();
        stack.reset(3);
        counters.insert(Symbol::new("chapter"), stack);
        let source = ContentSource::new(vec![ContentValueItem::Counter {
            name: Symbol::new("chapter"),
            style: CounterStyle::Decimal,
        }]);
        assert_eq!(
            resolve_content_source(&source, &counters),
            Some("3".to_owned())
        );
    }

    #[test]
    fn counter_item_absent_counter_formats_zero() {
        let counters = HashMap::new();
        let source = ContentSource::new(vec![ContentValueItem::Counter {
            name: Symbol::new("missing"),
            style: CounterStyle::Decimal,
        }]);
        assert_eq!(
            resolve_content_source(&source, &counters),
            Some("0".to_owned())
        );
    }

    #[test]
    fn counter_item_named_style_reaches_format_counter() {
        // Regression check for the format_counter/join_counter_stack
        // pub(crate) visibility widening: CounterStyle::Decimal alone
        // would exercise only
        // format_decimal, never touching the widened-visibility path. A
        // Named style forces resolve_content_source through
        // format_named_counter, proving the widening actually pays off
        // (not just compiles).
        let mut counters = HashMap::new();
        let mut stack = CounterStack::default();
        stack.reset(3);
        counters.insert(Symbol::new("chapter"), stack);
        let source = ContentSource::new(vec![ContentValueItem::Counter {
            name: Symbol::new("chapter"),
            style: CounterStyle::Named("upper-roman".into()),
        }]);
        assert_eq!(
            resolve_content_source(&source, &counters),
            Some("III".to_owned())
        );
    }

    #[test]
    fn counters_item_joins_full_stack_with_separator() {
        let mut counters = HashMap::new();
        let mut stack = CounterStack::default();
        stack.reset(1);
        stack.reset(1);
        counters.insert(Symbol::new("sec"), stack);
        let source = ContentSource::new(vec![ContentValueItem::Counters {
            name: Symbol::new("sec"),
            separator: ".".into(),
            style: CounterStyle::Decimal,
        }]);
        assert_eq!(
            resolve_content_source(&source, &counters),
            Some("1.1".to_owned())
        );
    }

    #[test]
    fn mixed_literal_and_counter_items_concatenate() {
        let mut counters = HashMap::new();
        let mut stack = CounterStack::default();
        stack.reset(2);
        counters.insert(Symbol::new("chapter"), stack);
        let source = ContentSource::new(vec![
            ContentValueItem::Literal("Ch. ".into()),
            ContentValueItem::Counter {
                name: Symbol::new("chapter"),
                style: CounterStyle::Decimal,
            },
        ]);
        assert_eq!(
            resolve_content_source(&source, &counters),
            Some("Ch. 2".to_owned())
        );
    }

    #[test]
    fn unresolvable_variant_yields_none_not_fabricated_empty_string() {
        // See resolve_content_source's doc "Skipping, not fabricating":
        // Some("") and None are NOT interchangeable under CSS GCPM 3
        // §1.1.2's string() keyword rules, so an unresolvable item must
        // skip the whole assignment (None), never contribute "".
        let counters = HashMap::new();
        let source = ContentSource::new(vec![
            ContentValueItem::Literal("[".into()),
            ContentValueItem::Attr {
                name: Symbol::new("data-title"),
            },
            ContentValueItem::Literal("]".into()),
        ]);
        assert_eq!(
            resolve_content_source(&source, &counters),
            None,
            "Attr needs DOM element access unavailable here — the whole \
             content-list must be skipped, not partially resolved to \"[]\""
        );
    }
}

// ── PageContext::apply_directive ──────────────────────

mod apply_directive_tests {
    use super::*;

    #[test]
    fn counter_directives_dispatch_to_named_stack() {
        let mut ctx = PageContext::default();
        let foo = Symbol::new("foo");
        let bar = Symbol::new("bar");
        ctx.apply_directive(&GcpmDirective::CounterReset {
            name: foo.clone(),
            value: 0,
        });
        ctx.apply_directive(&GcpmDirective::CounterIncrement {
            name: foo.clone(),
            delta: 3,
        });
        ctx.apply_directive(&GcpmDirective::CounterSet {
            name: bar.clone(),
            value: 9,
        });

        assert_eq!(ctx.counter(&foo).and_then(CounterStack::current), Some(3));
        assert_eq!(ctx.counter(&bar).and_then(CounterStack::current), Some(9));
    }

    #[test]
    fn same_element_reset_increment_set_order_matches_e81n_push_order() {
        let mut ctx = PageContext::default();
        let c = Symbol::new("c");
        let directives = vec![
            GcpmDirective::CounterReset {
                name: c.clone(),
                value: 0,
            },
            GcpmDirective::CounterIncrement {
                name: c.clone(),
                delta: 1,
            },
            GcpmDirective::CounterSet {
                name: c.clone(),
                value: 5,
            },
        ];
        for d in &directives {
            ctx.apply_directive(d);
        }
        assert_eq!(ctx.counter(&c).and_then(CounterStack::current), Some(5));
    }

    #[test]
    fn register_running_inserts_binding() {
        let mut ctx = PageContext::default();
        let name = Symbol::new("header");
        let id = RunningTemplateId::new(NodeId::new(7));
        ctx.apply_directive(&GcpmDirective::RegisterRunning {
            name: name.clone(),
            template_id: id,
        });
        assert_eq!(ctx.running_binding(&name), Some(id));
    }

    #[test]
    fn register_running_rebind_overwrites_previous_binding() {
        let mut ctx = PageContext::default();
        let name = Symbol::new("header");
        let id1 = RunningTemplateId::new(NodeId::new(7));
        let id2 = RunningTemplateId::new(NodeId::new(42));
        ctx.apply_directive(&GcpmDirective::RegisterRunning {
            name: name.clone(),
            template_id: id1,
        });
        ctx.apply_directive(&GcpmDirective::RegisterRunning {
            name: name.clone(),
            template_id: id2,
        });
        assert_eq!(
            ctx.running_binding(&name),
            Some(id2),
            "a later RegisterRunning for the same name must rebind, not be ignored"
        );
    }

    #[test]
    fn string_set_resolves_against_counter_values_at_apply_time() {
        let mut ctx = PageContext::default();
        let chapter = Symbol::new("chapter");
        let title = Symbol::new("title");

        ctx.apply_directive(&GcpmDirective::CounterReset {
            name: chapter.clone(),
            value: 3,
        });
        ctx.apply_directive(&GcpmDirective::StringSet {
            name: title.clone(),
            source: ContentSource::new(vec![
                ContentValueItem::Literal("Ch. ".into()),
                ContentValueItem::Counter {
                    name: chapter.clone(),
                    style: CounterStyle::Decimal,
                },
            ]),
        });

        // Mutate the counter AFTER the string-set fired.
        ctx.apply_directive(&GcpmDirective::CounterIncrement {
            name: chapter.clone(),
            delta: 100,
        });

        assert_eq!(
            ctx.string_state(&title).and_then(NamedStringState::running),
            Some("Ch. 3"),
            "resolved text must reflect the value AT string-set time (3), \
             not the later mutation (103)"
        );
        assert_eq!(
            ctx.counter(&chapter).and_then(CounterStack::current),
            Some(103)
        );
    }

    #[test]
    fn string_set_with_unresolvable_item_leaves_no_tracked_state() {
        // Regression check for the "skip, don't fabricate Some(\"\")"
        // fix — see resolve_content_source's doc "Skipping, not
        // fabricating" for why Some("") would be observably wrong
        // (CSS GCPM 3 §1.1.2 string() keyword fallback rules).
        let mut ctx = PageContext::default();
        let title = Symbol::new("title");
        ctx.apply_directive(&GcpmDirective::StringSet {
            name: title.clone(),
            source: ContentSource::new(vec![ContentValueItem::Attr {
                name: Symbol::new("data-title"),
            }]),
        });
        assert_eq!(
            ctx.string_state(&title),
            None,
            "an unresolvable string-set must not create a tracked entry at all"
        );
    }

    #[test]
    fn string_set_with_unresolvable_item_does_not_clobber_prior_resolvable_assignment() {
        let mut ctx = PageContext::default();
        let title = Symbol::new("title");
        ctx.apply_directive(&GcpmDirective::StringSet {
            name: title.clone(),
            source: ContentSource::new(vec![ContentValueItem::Literal("Ch. 1".into())]),
        });
        ctx.apply_directive(&GcpmDirective::StringSet {
            name: title.clone(),
            source: ContentSource::new(vec![ContentValueItem::Attr {
                name: Symbol::new("data-title"),
            }]),
        });
        assert_eq!(
            ctx.string_state(&title).and_then(NamedStringState::running),
            Some("Ch. 1"),
            "a later unresolvable string-set must skip entirely, not overwrite \
             the prior resolvable assignment with fabricated empty text"
        );
    }

    #[test]
    fn begin_page_forwards_to_every_tracked_named_string() {
        let mut ctx = PageContext::default();
        let a = Symbol::new("a");
        ctx.apply_directive(&GcpmDirective::StringSet {
            name: a.clone(),
            source: ContentSource::new(vec![ContentValueItem::Literal("A".into())]),
        });

        ctx.begin_page(1);

        assert!(ctx.string_state(&a).unwrap().on_page_start().is_some());
        assert!(ctx.string_state(&a).unwrap().on_page_first_use().is_none());
    }

    #[test]
    fn register_target_registers_live_counter_snapshot() {
        let mut ctx = PageContext::default();
        ctx.apply_directive(&GcpmDirective::CounterReset {
            name: Symbol::new("chapter"),
            value: 1,
        });
        ctx.apply_directive(&GcpmDirective::RegisterTarget {
            fragment_id: Symbol::new("intro"),
        });

        let mut registry = ctx.targets().clone();
        let out = registry.resolve_target_counter(
            "#intro",
            Symbol::new("chapter"),
            CounterStyle::Decimal,
        );
        assert_eq!(out, crate::page::ResolveOutcome::Resolved("1".to_owned()));
    }

    #[test]
    fn register_target_unlike_pre_promotion_dom_local_no_op_really_mutates_targets() {
        // Regression check against the pre-promotion dom-local behavior
        // (the deleted raikiri_dom::gcpm::PhaseBWalkState treated
        // RegisterTarget as a documented no-op) — this promoted
        // apply_directive must NOT preserve that no-op; it owns
        // TargetRegistry for real now.
        let before = PageContext::default();
        let mut after = before.clone();
        after.apply_directive(&GcpmDirective::RegisterTarget {
            fragment_id: Symbol::new("intro"),
        });

        let mut before_registry = before.targets().clone();
        let mut after_registry = after.targets().clone();
        let before_out = before_registry
            .resolve_target_text("#intro", raikiri_style::property::ContentPart::Content);
        let after_out = after_registry
            .resolve_target_text("#intro", raikiri_style::property::ContentPart::Content);
        assert_ne!(
            before_out, after_out,
            "RegisterTarget must observably register the fragment (Pending → Resolved), \
             unlike the dom-local no-op"
        );
    }

    #[test]
    fn register_target_does_not_clobber_a_richer_set_targets_registration() {
        // First-wins semantics (TargetRegistry::register) should mean a
        // fuller registration via set_targets survives a later
        // counts-only apply_directive RegisterTarget for the same
        // fragment_id — see apply_directive's RegisterTarget doc.
        let mut registry = TargetRegistry::new();
        let mut rich_info = TargetInfo::new();
        rich_info.text_parts.push((
            raikiri_style::property::ContentPart::Content,
            "Real Title".to_owned(),
        ));
        registry.register(Symbol::new("intro"), rich_info);

        let mut ctx = PageContext::default();
        ctx.set_targets(registry);
        ctx.apply_directive(&GcpmDirective::RegisterTarget {
            fragment_id: Symbol::new("intro"),
        });

        let mut result_registry = ctx.targets().clone();
        let out = result_registry
            .resolve_target_text("#intro", raikiri_style::property::ContentPart::Content);
        assert_eq!(
            out,
            crate::page::ResolveOutcome::Resolved("Real Title".to_owned()),
            "set_targets's richer TargetInfo must survive the later counts-only RegisterTarget"
        );
    }
}

// ── PageContext::pop_counter_scope ──────────────────────

mod pop_counter_scope_tests {
    use super::*;

    #[test]
    fn pops_a_scope_pushed_via_counter_reset_directive() {
        let mut ctx = PageContext::default();
        let c = Symbol::new("c");
        ctx.apply_directive(&GcpmDirective::CounterReset {
            name: c.clone(),
            value: 0,
        });
        ctx.apply_directive(&GcpmDirective::CounterReset {
            name: c.clone(),
            value: 10,
        });
        ctx.apply_directive(&GcpmDirective::CounterIncrement {
            name: c.clone(),
            delta: 5,
        });
        assert_eq!(ctx.counter(&c).and_then(CounterStack::current), Some(15));

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.pop_counter_scope(&c),
            Some(15),
            "must return the innermost (just-popped) scope's value"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.counter(&c).and_then(CounterStack::current),
            Some(0),
            "after popping the inner scope, a subsequent read must fall back to \
             the outer scope pushed by the first CounterReset"
        );
    }

    #[test]
    fn popping_the_only_scope_falls_back_to_absent() {
        let mut ctx = PageContext::default();
        let c = Symbol::new("c");
        ctx.apply_directive(&GcpmDirective::CounterReset {
            name: c.clone(),
            value: 3,
        });
        assert_eq!(ctx.pop_counter_scope(&c), Some(3));
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.counter(&c).and_then(CounterStack::current),
            None,
            "popping the only tracked scope must leave the counter reading as \
             absent, not silently resurrect a value"
        );
    }

    #[test]
    fn popping_a_name_with_no_tracked_counter_returns_none() {
        let mut ctx = PageContext::default();
        let untouched = Symbol::new("never-reset");
        assert_eq!(ctx.pop_counter_scope(&untouched), None);
    }
}

// ── PageContext::begin_page ────────────────────────────

mod begin_page_tests {
    use super::*;

    #[test]
    fn begin_page_updates_the_page_index_field() {
        let mut ctx = PageContext::default();
        assert_eq!(ctx.page_index, 0);
        ctx.begin_page(3);
        assert_eq!(ctx.page_index, 3);
    }

    #[test]
    fn begin_page_forwards_page_index_to_the_targets_registry() {
        // The coordination this method exists for: a slot minted after
        // begin_page(7) must be stamped with page_index 7, not the
        // registry's stale prior value — that staleness is exactly the
        // desync a bare `ctx.page_index = 7` write (with no registry
        // call) would leave behind.
        let mut ctx = PageContext::default();
        ctx.begin_page(7);

        let mut registry = ctx.targets().clone();
        let out = registry.resolve_target_counter(
            "#not-yet-registered",
            Symbol::new("chapter"),
            CounterStyle::Decimal,
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            out,
            crate::page::ResolveOutcome::Pending(crate::error::TargetSlotId {
                page_index: 7,
                sequence: 0,
            }),
            "a slot minted after begin_page(7) must be stamped with page_index 7"
        );
    }

    #[test]
    fn begin_page_resets_the_registrys_page_local_sequence_counter() {
        let mut ctx = PageContext::default();

        // Mint a slot on page 0 to advance next_sequence past 0.
        let mut registry = ctx.targets().clone();
        registry.resolve_target_counter("#a", Symbol::new("c"), CounterStyle::Decimal);
        ctx.set_targets(registry);

        ctx.begin_page(1);

        let mut registry = ctx.targets().clone();
        let out = registry.resolve_target_counter("#b", Symbol::new("c"), CounterStyle::Decimal);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            out,
            crate::page::ResolveOutcome::Pending(crate::error::TargetSlotId {
                page_index: 1,
                sequence: 0,
            }),
            "the page-local sequence must restart at 0 on a new page, not \
             continue from page 0's count"
        );
    }

    #[test]
    #[should_panic(expected = "monotonically")]
    fn begin_page_backward_call_panics_through_page_context() {
        // TargetRegistry::begin_page's own monotonic-page_index assert
        // must be reachable through this coordinating method, not just
        // when TargetRegistry is driven directly.
        let mut ctx = PageContext::default();
        ctx.begin_page(5);
        ctx.begin_page(3);
    }

    #[test]
    fn begin_page_backward_call_panic_leaves_page_index_unchanged() {
        // Registry-first call order (see begin_page's doc "Call order
        // inside this method is registry-first"): the assert! in
        // TargetRegistry::begin_page fires before self.page_index is
        // reassigned, so a caught panic must leave page_index paired
        // with the *last successful* call, not torn (advanced) while
        // targets stayed on the old page.
        let mut ctx = PageContext::default();
        ctx.begin_page(5);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ctx.begin_page(3);
        }));
        assert!(result.is_err(), "backward begin_page call must panic");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            ctx.page_index, 5,
            "a panicking begin_page call must not partially apply — page_index \
             must remain paired with the last successful call"
        );
    }
}

// ── PageContext::set_targets ──────────────────────────

mod set_targets_tests {
    use super::*;

    #[test]
    fn set_targets_replaces_the_registry_wholesale() {
        let mut registry = TargetRegistry::new();
        registry.register(Symbol::new("ch1"), TargetInfo::new());

        let mut ctx = PageContext::default();
        ctx.set_targets(registry);

        let mut result = ctx.targets().clone();
        let out = result.resolve_target_counter(
            "#ch1",
            Symbol::new("nonexistent"),
            CounterStyle::Decimal,
        );
        // Missing counter on an existing (registered) target resolves
        // immediately to "0", proving #ch1 landed via set_targets.
        assert_eq!(out, crate::page::ResolveOutcome::Resolved("0".to_owned()));
    }

    #[test]
    fn set_targets_after_register_target_wipes_the_earlier_registration() {
        // Regression check: the apply_directive RegisterTarget arm's doc
        // documents that
        // set_targets called AFTER a prior RegisterTarget directive is a
        // wholesale wipe, not an order-independent merge — only the
        // reverse ordering (set_targets first) had a test. This pins the
        // other direction: a fragment registered via apply_directive
        // must NOT survive a later set_targets call whose registry
        // doesn't contain it.
        let mut ctx = PageContext::default();
        ctx.apply_directive(&GcpmDirective::CounterReset {
            name: Symbol::new("chapter"),
            value: 1,
        });
        ctx.apply_directive(&GcpmDirective::RegisterTarget {
            fragment_id: Symbol::new("intro"),
        });

        // A fresh, unrelated registry that never saw "intro".
        ctx.set_targets(TargetRegistry::new());

        let mut result = ctx.targets().clone();
        let out =
            result.resolve_target_counter("#intro", Symbol::new("chapter"), CounterStyle::Decimal);
        assert_eq!(
            out,
            crate::page::ResolveOutcome::Pending(crate::error::TargetSlotId {
                page_index: 0,
                sequence: 0,
            }),
            "set_targets after RegisterTarget must wipe the earlier registration, \
             not merge with or preserve it"
        );
    }
}

// ── page_index / page_name plain fields ───────────────

mod plain_field_tests {
    use super::*;

    #[test]
    fn page_index_defaults_to_zero_and_is_directly_settable() {
        let mut ctx = PageContext::default();
        assert_eq!(ctx.page_index, 0);
        ctx.page_index = 5;
        assert_eq!(ctx.page_index, 5);
    }

    #[test]
    fn page_name_defaults_to_none_and_is_directly_settable() {
        let mut ctx = PageContext::default();
        assert_eq!(ctx.page_name, None);
        ctx.page_name = Some(Symbol::new("chapter-front"));
        assert_eq!(ctx.page_name, Some(Symbol::new("chapter-front")));
    }
}

// ── promotion parity ──────────────────────────────────

#[test]
fn page_context_new_and_default_agree_with_umbrella_contract() {
    // crates/raikiri/tests/external_consumer.rs and strategy.rs's
    // TargetResolver already depend on both constructors continuing to
    // work across this promotion.
    let a = PageContext::new();
    let b = PageContext::default();
    assert_eq!(a.page_index, b.page_index);
    assert_eq!(a.page_name, b.page_name);
}
