use super::*;

#[test]
fn page_completion_releases_weakref_kept_objects_on_success_and_error() {
    for script in ["JSON.stringify([])", "throw new Error('page failure')"] {
        let mut worker = Worker::new().unwrap();
        let queue = v8::MicrotaskQueue::new(&mut worker.isolate, v8::MicrotasksPolicy::Explicit);
        let weak = {
            v8::scope!(let scope, &mut worker.isolate);
            let context = v8::Context::new(scope, Default::default());
            let default_queue = context.get_microtask_queue().unwrap();
            context.set_microtask_queue(queue.as_ref());
            let weak = {
                let scope = &mut v8::ContextScope::new(scope, context);
                let target = v8::Object::new(scope);
                let key = v8::String::new(scope, "target").unwrap();
                assert_eq!(
                    context.global(scope).set(scope, key.into(), target.into()),
                    Some(true)
                );
                let source =
                    v8::String::new(scope, "new WeakRef(target); delete globalThis.target;")
                        .unwrap();
                v8::Script::compile(scope, source, None)
                    .unwrap()
                    .run(scope)
                    .unwrap();
                v8::Weak::new(scope, target)
            };
            context.set_microtask_queue(default_queue);
            weak
        };
        drop(queue);
        worker
            .isolate
            .request_garbage_collection_for_testing(v8::GarbageCollectionType::Full);
        assert!(!weak.is_empty());
        let result = worker.run_page(script);
        assert_eq!(result.is_err(), script.starts_with("throw"));
        worker
            .isolate
            .request_garbage_collection_for_testing(v8::GarbageCollectionType::Full);
        assert!(weak.is_empty());
    }
}
