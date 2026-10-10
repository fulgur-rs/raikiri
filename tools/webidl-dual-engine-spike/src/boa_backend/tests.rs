use super::*;

#[derive(Debug, Trace, JsData)]
struct Probe {
    #[unsafe_ignore_trace]
    finalized: Rc<Cell<usize>>,
}

impl Finalize for Probe {
    fn finalize(&self) {
        self.finalized.set(self.finalized.get() + 1);
    }
}

#[test]
fn page_completion_releases_weakref_kept_objects_on_success_and_error() {
    for script in ["JSON.stringify([])", "throw new Error('page failure')"] {
        let mut worker = Worker::new().unwrap();
        let finalized = Rc::new(Cell::new(0));
        let realm = worker.context.create_realm().unwrap();
        let previous = worker.context.enter_realm(realm);
        let target = JsObject::from_proto_and_data(
            None,
            Probe {
                finalized: finalized.clone(),
            },
        );
        worker
            .context
            .register_global_property(
                JsString::from("target"),
                target,
                boa_engine::property::Attribute::all(),
            )
            .unwrap();
        worker
            .context
            .eval(Source::from_bytes(
                "new WeakRef(target); delete globalThis.target;",
            ))
            .unwrap();
        worker.context.enter_realm(previous);
        boa_gc::force_collect();
        assert_eq!(finalized.get(), 0);
        let result = worker.run_page(script);
        assert_eq!(result.is_err(), script.starts_with("throw"));
        boa_gc::force_collect();
        assert_eq!(finalized.get(), 1);
    }
}
