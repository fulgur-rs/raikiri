use super::StyleElement;

struct MinimalElement;

impl StyleElement for MinimalElement {
    fn tag_name(&self) -> &str {
        "div"
    }
}

#[test]
fn animation_style_defaults_to_absent() {
    assert_eq!(MinimalElement.animation_style_source(), None);
}
