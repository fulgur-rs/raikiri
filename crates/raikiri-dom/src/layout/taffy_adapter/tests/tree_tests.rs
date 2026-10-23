//! The library's tree traits on a layout session only convert their arguments
//! and hand them to the session.

use super::*;
use crate::document::Document;
use crate::layout::context::LayoutSession;
use crate::layout::test_support::cascaded;
use raikiri_layout_style::ViewContext;
use taffy::{CacheTree, LayoutContainingBlock};

#[test]
fn the_library_cache_calls_reach_the_cache_of_the_node() {
    let mut document = Document::new();
    let cascade = cascaded(&document);
    let mut session = LayoutSession::with_cascade(&mut document, &cascade);
    session.prepare_style_views();
    let node = taffy::NodeId::from(0_usize);
    let mut floats = Floats(2);
    let layout_input = LayoutInput {
        run_mode: RunMode::PerformLayout,
        ..layout_input(&mut floats)
    };
    let output = layout_output(&mut floats);

    session.with_cache_frame(0, ViewContext::default(), |session| {
        let taffy_input = layout_input.into_taffy();
        assert!(CacheTree::cache_get(session, node, &taffy_input).is_none());
        CacheTree::cache_store(session, node, &taffy_input, output.clone().into_taffy());
        let hit = CacheTree::cache_get(session, node, &taffy_input).expect("the stored layout");
        assert_eq!(bits_of(&LayoutOutput::from_taffy(hit)), bits_of(&output));
        // The session answers the same lookup through its own method.
        assert!(
            session
                .cache_get(NodeId::from_taffy(node), &layout_input)
                .is_some()
        );

        CacheTree::cache_clear(session, node);
        assert!(CacheTree::cache_get(session, node, &taffy_input).is_none());
    });
}

#[test]
fn clearing_hoisted_children_through_the_library_forgets_them() {
    let mut document = Document::new();
    let cascade = cascaded(&document);
    let mut session = LayoutSession::with_cascade(&mut document, &cascade);
    session
        .context
        .taffy_hoisted_children
        .insert(0, vec![NodeId::from_taffy(taffy::NodeId::from(1_usize))]);

    LayoutContainingBlock::clear_hoisted_children(&mut session, taffy::NodeId::from(0_usize));

    assert!(session.context.taffy_hoisted_children.is_empty());
}
