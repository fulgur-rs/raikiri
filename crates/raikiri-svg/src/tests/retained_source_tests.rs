use super::*;
use std::sync::Arc;

#[test]
fn a_source_only_document_drops_the_backend_tree_and_preserves_outputs() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><defs><path id="p" d="M0 0H10V10H0Z" fill="red"/></defs><use href="#p"/><use href="#p" x="10"/></svg>"##;
    let parsed = Arc::new(SvgDocument::parse(source).unwrap());
    assert!(parsed.source_only_retained_bytes().is_none());
    let compact = Arc::clone(&parsed).into_source_only();
    assert!(compact.tree.is_none());
    assert!(parsed.tree.is_some());
    assert!(compact.source_only_retained_bytes().unwrap() >= source.len() as u64);
    assert!(Arc::ptr_eq(
        &compact,
        &Arc::clone(&compact).into_source_only()
    ));
    let viewport = SvgViewport {
        width: 20.0,
        height: 10.0,
    };
    for viewport in [
        viewport,
        SvgViewport {
            width: 40.0,
            height: 15.0,
        },
    ] {
        assert_eq!(
            parsed
                .rasterize(viewport, SvgRootStyle::default(), None)
                .unwrap(),
            compact
                .rasterize(viewport, SvgRootStyle::default(), None)
                .unwrap(),
        );
        assert_eq!(
            format!(
                "{:?}",
                parsed.scene(viewport, SvgRootStyle::default()).unwrap()
            ),
            format!(
                "{:?}",
                compact.scene(viewport, SvgRootStyle::default()).unwrap()
            ),
        );
    }
}
