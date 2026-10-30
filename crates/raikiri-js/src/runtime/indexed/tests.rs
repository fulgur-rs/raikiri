use super::*;

struct NamedSource(bool);

impl IndexedSource for NamedSource {
    fn length(&self, _: &mut Context) -> JsResult<usize> {
        Ok(0)
    }

    fn item(&self, _: usize, _: &mut Context) -> JsResult<Option<JsValue>> {
        Ok(None)
    }

    fn named_property(&self, name: &str, _: &mut Context) -> JsResult<Option<JsValue>> {
        Ok((name == "length").then(|| 17.into()))
    }

    fn legacy_override_built_ins(&self) -> bool {
        self.0
    }
}

#[test]
fn named_read_respects_legacy_override_built_ins() {
    for (overrides, expected) in [(false, 99.0), (true, 17.0)] {
        let context = &mut Context::default();
        install(context).unwrap();
        let prototype = JsObject::with_null_proto();
        prototype
            .create_data_property_or_throw(js_string!("length"), 99, context)
            .unwrap();
        let object = indexed_object(context, prototype, NamedSource(overrides)).unwrap();
        assert_eq!(
            object
                .get(js_string!("length"), context)
                .unwrap()
                .as_number(),
            Some(expected)
        );
    }
}
