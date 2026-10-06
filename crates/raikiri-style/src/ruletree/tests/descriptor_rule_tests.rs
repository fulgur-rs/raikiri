use super::*;
use crate::counter_style::parse_counter_style_rules;
use crate::font_face::parse_font_face_rules;
use crate::media::MediaType;

fn descriptors(name: &str, marker: &str) -> String {
    format!(
        "@font-face {{font-family:{name};src:url({marker}.woff)}} \
         @counter-style {name} {{system:cyclic;symbols:\"{marker}\"}}"
    )
}

fn assert_registration(tree: &RuleTree, context: &MediaContext, name: &str, marker: Option<&str>) {
    let fonts = tree.font_faces_for(context);
    let counters = tree.counter_styles_for(context);
    if let Some(marker) = marker {
        let source = descriptors(name, marker);
        let expected_fonts = parse_font_face_rules(&source);
        let expected_counters = parse_counter_style_rules(&source);
        assert_eq!(expected_fonts.len(), 1);
        assert_eq!(expected_counters.len(), 1);
        assert_eq!(fonts.get(name), Some(&expected_fonts[0]));
        assert_eq!(counters.get(name), Some(&expected_counters[0]));
    } else {
        assert!(fonts.get(name).is_none());
        assert!(counters.get(name).is_none());
    }
}

#[test]
fn root_descriptors_keep_standalone_grammar_and_raw_records() {
    let source = descriptors("demo", "red");
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&source, Origin::User);
    assert_registration(&tree, &MediaContext::print(), "demo", Some("red"));
    assert_registration(&tree, &MediaContext::screen(), "demo", Some("red"));
    assert_eq!(tree.font_faces().len(), 1);
    assert_eq!(tree.counter_styles().len(), 1);
    assert_eq!(tree.opaque_at_rules().len(), 2);
    assert_eq!(tree.opaque_at_rules()[0].name, "font-face");
    assert_eq!(tree.opaque_at_rules()[1].name, "counter-style");
    assert!(
        tree.opaque_at_rules()
            .iter()
            .all(|record| record.origin == Origin::User)
    );
    let raw = "@FONT-FACE /* prelude */ { /* body */ font-family:demo;src:url(red.woff) }";
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(raw, Origin::Author);
    assert_eq!(tree.opaque_at_rules()[0].to_css(), raw);
}

#[test]
fn supported_descriptor_first_groups_register_without_style_rules() {
    let body = descriptors("demo", "red");
    for source in [
        format!("@supports (color:red) {{{body}}}"),
        format!("@supports (color:red) {{@supports (display:block) {{{body}}}}}"),
        format!("@supports selector(p) {{@supports (color:red) {{{body}}}}}"),
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&source, Origin::Author);
        assert_registration(&tree, &MediaContext::print(), "demo", Some("red"));
        assert_eq!(tree.font_faces().len(), 1, "{source}");
        assert_eq!(tree.counter_styles().len(), 1, "{source}");
    }
}

#[test]
fn media_descriptors_are_visible_only_in_the_matching_context() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        &format!(
            "@media print {{{}}} @media screen {{{}}}",
            descriptors("printed", "red"),
            descriptors("screened", "blue")
        ),
        Origin::Author,
    );
    assert!(tree.font_faces().is_empty());
    assert!(tree.counter_styles().is_empty());
    assert_registration(&tree, &MediaContext::print(), "printed", Some("red"));
    assert_registration(&tree, &MediaContext::screen(), "printed", None);
    assert_registration(&tree, &MediaContext::screen(), "screened", Some("blue"));
    assert_registration(&tree, &MediaContext::print(), "screened", None);
}

#[test]
fn media_counter_descriptors_register_without_a_font_rule() {
    let source = "@counter-style demo {system:cyclic;symbols:\"red\"}";
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&format!("@media print {{{source}}}"), Origin::Author);
    assert_eq!(
        tree.counter_styles_for(&MediaContext::print()).get("demo"),
        parse_counter_style_rules(source).first()
    );
    assert!(
        tree.counter_styles_for(&MediaContext::screen())
            .get("demo")
            .is_none()
    );
}

#[test]
fn descriptor_groups_combine_supports_and_media_in_either_order() {
    let body = descriptors("demo", "red");
    for source in [
        format!("@media print {{@supports (color:red) {{{body}}}}}"),
        format!("@supports (color:red) {{@media print {{{body}}}}}"),
        format!("@supports (color:red) {{@media print {{@supports (display:block) {{{body}}}}}}}"),
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&source, Origin::User);
        assert_registration(&tree, &MediaContext::print(), "demo", Some("red"));
        assert_registration(&tree, &MediaContext::screen(), "demo", None);
        assert!(tree.font_faces().is_empty());
        assert!(tree.counter_styles().is_empty());
    }
}

#[test]
fn descriptor_media_intersects_with_the_stylesheet_condition() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet_with_media(
        &format!(
            "@supports (color:red) {{@media (min-width:500px) {{{}}}}}",
            descriptors("demo", "red")
        ),
        Origin::Author,
        Some("print"),
    );
    for (kind, width, marker) in [
        (MediaType::Print, 600, Some("red")),
        (MediaType::Print, 400, None),
        (MediaType::Screen, 600, None),
    ] {
        assert_registration(
            &tree,
            &MediaContext::with_viewport(kind, width, 100),
            "demo",
            marker,
        );
    }
}

#[test]
fn conditional_same_name_registrations_keep_cross_group_source_order() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&descriptors("demo", "base"), Origin::Author);
    tree.add_stylesheet(
        &format!(
            "@media print {{@supports (color:red) {{{}}}}}",
            descriptors("demo", "printed")
        ),
        Origin::Author,
    );
    assert_registration(&tree, &MediaContext::print(), "demo", Some("printed"));
    assert_registration(&tree, &MediaContext::screen(), "demo", Some("base"));
    tree.add_stylesheet(
        &format!("@supports (color:red) {{{}}}", descriptors("demo", "last")),
        Origin::Author,
    );
    assert_registration(&tree, &MediaContext::print(), "demo", Some("last"));
    assert_registration(&tree, &MediaContext::screen(), "demo", Some("last"));
}

#[test]
fn descriptor_origin_rank_wins_over_later_conditional_registrations() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&descriptors("demo", "author"), Origin::Author);
    for origin in [Origin::User, Origin::UserAgent] {
        tree.add_stylesheet(
            &format!("@media print {{{}}}", descriptors("demo", "lower")),
            origin,
        );
    }
    assert_registration(&tree, &MediaContext::print(), "demo", Some("author"));
    tree.add_stylesheet(
        &format!("@media print {{{}}}", descriptors("demo", "later")),
        Origin::Author,
    );
    assert_registration(&tree, &MediaContext::print(), "demo", Some("later"));
    assert_registration(&tree, &MediaContext::screen(), "demo", Some("author"));
}

#[test]
fn false_invalid_and_unknown_groups_never_register_descriptors() {
    let body = descriptors("hidden", "red");
    for source in [
        format!("@supports (display:invalid) {{{body}}}"),
        format!("@supports (color:red) or (color:blue) and (display:block) {{{body}}}"),
        format!("@media not all {{{body}}}"),
        format!("@media print, {{{body}}}"),
        format!("@supports (color:red) {{@media not all {{{body}}}}}"),
        format!("@media print {{@supports (display:invalid) {{{body}}}}}"),
        format!("@future {{@supports (color:red) {{{body}}}}}"),
        format!("@media print {{@future {{{body}}}}}"),
        format!("p {{{body}}}"),
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&source, Origin::Author);
        assert_registration(&tree, &MediaContext::print(), "hidden", None);
        assert_registration(&tree, &MediaContext::screen(), "hidden", None);
    }
}

#[test]
fn invalid_descriptor_rules_do_not_hide_valid_siblings_or_raw_records() {
    let invalid = "@font-face named {font-family:hidden;src:url(hidden.woff)} \
        @counter-style decimal {system:cyclic;symbols:\"bad\"} \
        @font-face; @counter-style hidden; \
        @font-face {font-family:missing} \
        @counter-style missing {system:numeric;symbols:\"bad\"}";
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(invalid, Origin::Author);
    assert!(tree.font_faces().is_empty());
    assert!(tree.counter_styles().is_empty());
    assert_eq!(tree.opaque_at_rules().len(), 6);
    tree.add_stylesheet(
        &format!("@media print {{{invalid} {}}}", descriptors("demo", "red")),
        Origin::Author,
    );
    assert_registration(&tree, &MediaContext::print(), "demo", Some("red"));
    assert_registration(&tree, &MediaContext::screen(), "demo", None);
}

#[test]
fn descriptor_error_recovery_keeps_later_valid_values() {
    let source = "@font-face {font-family:demo;font-weight:\"bad\n;src:url(red.woff)} \
        @counter-style demo {system:cyclic;symbols:\"bad\n;symbols:\"red\"}";
    for wrapper in [None, Some("@media print"), Some("@supports (color:red)")] {
        let source = wrapper.map_or_else(
            || source.to_owned(),
            |wrapper| format!("{wrapper} {{{source}}}"),
        );
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&source, Origin::Author);
        assert_registration(&tree, &MediaContext::print(), "demo", Some("red"));
        assert_eq!(tree.opaque_at_rules().len(), usize::from(wrapper.is_some()));
    }
}

#[test]
fn root_descriptor_eof_recovery_keeps_existing_registration_behavior() {
    for source in [
        "@font-face {font-family:demo;src:url(red.woff)",
        "@counter-style demo {system:cyclic;symbols:\"red\"",
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(source, Origin::Author);
        let fonts = parse_font_face_rules(source);
        let counters = parse_counter_style_rules(source);
        assert_eq!(fonts.len() + counters.len(), 1);
        assert_eq!(tree.font_faces().get("demo"), fonts.first());
        assert_eq!(tree.counter_styles().get("demo"), counters.first());
        assert!(tree.opaque_at_rules().is_empty());
    }
}

#[test]
fn descriptor_conditions_share_registered_consumer_properties() {
    let properties = [ConsumerPropertyRegistration::integer("bookmark-level")];
    let mut tree = RuleTree::empty_with_consumer_properties(&properties);
    tree.add_stylesheet(
        &format!(
            "@supports (bookmark-level:2) {{@media print {{{}}}}}",
            descriptors("demo", "red")
        ),
        Origin::Author,
    );
    assert_registration(&tree, &MediaContext::print(), "demo", Some("red"));
    assert_registration(&tree, &MediaContext::screen(), "demo", None);
}

#[test]
fn escaped_descriptor_and_group_names_use_css_tokens() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        r#"@\73 upports (color:red) {@\6d edia print {
        @\66 ont-face {font-family:demo;src:url(red.woff)}
        @\63 ounter-style \64 emo {system:cyclic;symbols:"red"}
    }}"#,
        Origin::Author,
    );
    assert_registration(&tree, &MediaContext::print(), "demo", Some("red"));
    assert_registration(&tree, &MediaContext::screen(), "demo", None);
}

#[test]
fn descriptor_groups_respect_depth_and_unclosed_container_limits() {
    let body = descriptors("demo", "red");
    for (depth, expected) in [(2, Some("red")), (MAX_OPAQUE_RULE_NESTING_DEPTH + 2, None)] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            &format!(
                "{}{body}{}",
                "@supports (color:red) {".repeat(depth),
                "}".repeat(depth)
            ),
            Origin::Author,
        );
        assert_registration(&tree, &MediaContext::print(), "demo", expected);
    }
    for source in [
        format!("@media print {{{body}"),
        format!("@supports (color:red) {{{body}"),
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&source, Origin::Author);
        assert_registration(&tree, &MediaContext::print(), "demo", None);
        assert!(tree.opaque_at_rules().is_empty());
    }
}

#[test]
fn descriptor_registration_keeps_layer_chunk_precedence_and_media() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        &format!(
            "{} @layer base {{{}}}",
            descriptors("demo", "outside"),
            descriptors("demo", "layered")
        ),
        Origin::Author,
    );
    assert_registration(&tree, &MediaContext::print(), "demo", Some("outside"));
    tree.add_stylesheet(
        &format!(
            "@layer other {{@supports (color:red) {{@media print {{{}}}}}}}",
            descriptors("demo", "printed")
        ),
        Origin::Author,
    );
    assert_registration(&tree, &MediaContext::print(), "demo", Some("printed"));
    assert_registration(&tree, &MediaContext::screen(), "demo", Some("outside"));
}
