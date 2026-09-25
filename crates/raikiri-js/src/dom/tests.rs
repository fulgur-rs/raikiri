use super::*;

struct LegacyBackend;

impl DomBackend for LegacyBackend {
    fn get_element_by_id(&mut self, id: &str) -> Result<Option<DomNodeId>, String> {
        Ok((id == "target").then_some(1))
    }

    fn query_selector(&mut self, _selector: &str) -> Result<Option<DomNodeId>, String> {
        Ok(None)
    }

    fn parent_node(&mut self, _node: DomNodeId) -> Result<Option<DomNodeId>, String> {
        Ok(None)
    }

    fn offset_height(&mut self, _node: DomNodeId) -> Result<f64, String> {
        Ok(0.0)
    }

    fn bounding_client_rect(&mut self, _node: DomNodeId) -> Result<DomRect, String> {
        Ok(DomRect::default())
    }

    fn inner_html(&mut self, _node: DomNodeId) -> Result<String, String> {
        Ok(String::new())
    }

    fn set_inner_html(&mut self, _node: DomNodeId, _value: &str) -> Result<(), String> {
        Ok(())
    }

    fn style_property(&mut self, _node: DomNodeId, _property: &str) -> Result<String, String> {
        Ok(String::new())
    }

    fn computed_style_property(
        &mut self,
        _node: DomNodeId,
        property: &str,
    ) -> Result<Option<String>, String> {
        Ok((property == "white-space").then(|| "pre".to_owned()))
    }

    fn set_style_property(
        &mut self,
        _node: DomNodeId,
        _property: &str,
        _value: &str,
    ) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn optional_attribute_methods_keep_legacy_backends_compatible() {
    let mut backend = LegacyBackend;
    assert_eq!(backend.get_element_by_id("missing").unwrap(), None);
    assert_eq!(backend.query_selector("#missing").unwrap(), None);
    assert_eq!(backend.parent_node(1).unwrap(), None);
    assert_eq!(backend.offset_height(1).unwrap(), 0.0);
    assert_eq!(backend.bounding_client_rect(1).unwrap(), DomRect::default());
    assert_eq!(backend.inner_html(1).unwrap(), "");
    backend.set_inner_html(1, "content").unwrap();
    assert_eq!(backend.style_property(1, "color").unwrap(), "");
    backend.set_style_property(1, "color", "red").unwrap();

    assert!(backend.create_element("div").is_err());
    assert!(backend.append_child(1, 2).is_err());
    assert!(backend.text_content(1).is_err());
    assert!(backend.set_text_content(1, "text").is_err());
    assert!(backend.get_attribute(1, "id").is_err());
    assert!(backend.has_attribute(1, "id").is_err());
    assert!(backend.set_attribute(1, "id", "value").is_err());
    assert!(backend.remove_attribute(1, "id").is_err());
}

#[test]
fn computed_style_and_css_supports_are_available_in_testharness_scripts() {
    let outcomes = crate::testharness::run_testharness_script(
        r#"
        test(function () {
            var target = document.getElementById("target");
            var style = getComputedStyle(target);
            assert_true("white-space" in style);
            assert_equals(style["white-space"], "pre");
            assert_equals(style.getPropertyValue("white-space"), "pre");
            assert_true(CSS.supports("white-space", "break-spaces"));
            assert_equals(CSS.supports("white-space", "not-a-white-space-value"), false);
            assert_true(CSS.supports("text-autospace", "initial"));
            assert_equals(CSS.supports("not-a-property", "initial"), false);
        }, "computed style values and CSS.supports use the DOM/CSS adapters");
        "#,
        LegacyBackend,
    )
    .unwrap();

    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].passed, "{}", outcomes[0].message);
}
