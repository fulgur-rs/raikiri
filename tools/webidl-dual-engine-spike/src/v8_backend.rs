use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Once;

use crate::shared::{BindingCall, Dom, DomError, Interface, NativeValue, PageOutput};

struct State {
    dom: Dom,
    prototypes: HashMap<String, v8::Global<v8::Object>>,
    constructors: HashMap<String, v8::Global<v8::Function>>,
    wrappers: HashMap<usize, v8::Global<v8::Object>>,
}
type Shared = Rc<RefCell<State>>;

#[cfg(test)]
#[path = "v8_backend/tests.rs"]
mod tests;
fn state(scope: &v8::PinScope<'_, '_>) -> Shared {
    scope
        .get_current_context()
        .get_slot::<RefCell<State>>()
        .expect("installed state")
}

pub(crate) struct Worker {
    isolate: v8::OwnedIsolate,
    job_witness: Rc<Cell<usize>>,
}

fn checkpoint(
    scope: &mut v8::PinScope<'_, '_>,
    _: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    let context = scope.get_current_context();
    context
        .get_microtask_queue()
        .expect("page queue")
        .perform_checkpoint(scope);
}

fn mark_job(
    scope: &mut v8::PinScope<'_, '_>,
    _: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    let witness = scope.get_slot::<Rc<Cell<usize>>>().expect("worker witness");
    witness.set(witness.get() + 1);
}
fn name(interface: Interface) -> &'static str {
    match interface {
        Interface::Node => "Node",
        Interface::Element => "Element",
    }
}

enum Error {
    Type(&'static str),
    Dom(String),
    Pending,
}

fn handle(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<v8::Value>,
    interface: Interface,
) -> Result<usize, Error> {
    let object =
        v8::Local::<v8::Object>::try_from(value).map_err(|_| Error::Type("Illegal invocation"))?;
    if object.internal_field_count() != 1 {
        return Err(Error::Type("Illegal invocation"));
    }
    let field = object
        .get_internal_field(scope, 0)
        .ok_or(Error::Type("Illegal invocation"))?;
    let index = v8::Local::<v8::Integer>::try_from(field)
        .map_err(|_| Error::Type("Illegal invocation"))?
        .value() as usize;
    if !state(scope).borrow().dom.supports(index, interface) {
        return Err(Error::Type("Illegal invocation"));
    }
    Ok(index)
}

fn wrap<'s>(scope: &v8::PinScope<'s, '_>, index: usize) -> v8::Local<'s, v8::Object> {
    let state = state(scope);
    let mut state = state.borrow_mut();
    if let Some(object) = state.wrappers.get(&index) {
        return v8::Local::new(scope, object);
    }
    let template = v8::ObjectTemplate::new(scope);
    assert!(template.set_internal_field_count(1));
    let object = template.new_instance(scope).expect("node wrapper");
    let field = v8::Integer::new_from_unsigned(scope, index.try_into().expect("fixture index"));
    assert!(object.set_internal_field(0, field.into()));
    let prototype = v8::Local::new(scope, &state.prototypes[name(state.dom.interface(index))]);
    assert_eq!(object.set_prototype(scope, prototype.into()), Some(true));
    state.wrappers.insert(index, v8::Global::new(scope, object));
    object
}

struct Call<'a, 's, 'i> {
    scope: &'a mut v8::PinScope<'s, 'i>,
    args: v8::FunctionCallbackArguments<'s>,
}

impl<'s> BindingCall for Call<'_, 's, '_> {
    type Value = v8::Local<'s, v8::Value>;
    type Error = Error;
    fn receiver(&mut self, interface: Interface) -> Result<usize, Error> {
        handle(self.scope, self.args.this().into(), interface)
    }
    fn require(&mut self, count: usize) -> Result<(), Error> {
        if (self.args.length() as usize) < count {
            Err(Error::Type("required argument missing"))
        } else {
            Ok(())
        }
    }
    fn dom_string(&mut self, index: usize) -> Result<String, Error> {
        let string = self
            .args
            .get(index as i32)
            .to_string(self.scope)
            .ok_or(Error::Pending)?;
        let mut units = vec![0; string.length()];
        string.write_v2(self.scope, 0, &mut units, v8::WriteFlags::empty());
        String::from_utf16(&units)
            .map_err(|_| Error::Type("prototype DOM storage requires valid Unicode"))
    }
    fn nullable_node(&mut self, index: usize) -> Result<Option<usize>, Error> {
        let value = self.args.get(index as i32);
        if value.is_null_or_undefined() {
            Ok(None)
        } else {
            handle(self.scope, value, Interface::Node).map(Some)
        }
    }
    fn with_dom<R>(&mut self, f: impl FnOnce(&mut Dom) -> R) -> R {
        f(&mut state(self.scope).borrow_mut().dom)
    }
    fn native_error(&mut self, error: DomError) -> Error {
        Error::Dom(error.0)
    }
    fn output(&mut self, value: NativeValue) -> Result<Self::Value, Error> {
        Ok(match value {
            NativeValue::Number(v) => {
                v8::Integer::new_from_unsigned(self.scope, u32::from(v)).into()
            }
            NativeValue::Boolean(v) => v8::Boolean::new(self.scope, v).into(),
            NativeValue::String(Some(v)) => v8::String::new(self.scope, &v)
                .expect("return string")
                .into(),
            NativeValue::Node(Some(v)) => wrap(self.scope, v).into(),
            NativeValue::String(None) | NativeValue::Node(None) => v8::null(self.scope).into(),
            NativeValue::Undefined(()) => v8::undefined(self.scope).into(),
        })
    }
}

fn throw(scope: &v8::PinScope<'_, '_>, error: Error) {
    let (message, type_error) = match error {
        Error::Pending => return,
        Error::Type(m) => (m.to_owned(), true),
        Error::Dom(m) => (m, false),
    };
    let message = v8::String::new(scope, &message).expect("error message");
    let exception = if type_error {
        v8::Exception::type_error(scope, message)
    } else {
        v8::Exception::error(scope, message)
    };
    if !type_error {
        let object = v8::Local::<v8::Object>::try_from(exception).expect("error object");
        let key = v8::String::new(scope, "name").expect("error key");
        let value = v8::String::new(scope, "InvalidCharacterError").expect("error name");
        define(scope, object, key.into(), value.into(), true, false, true);
    }
    scope.throw_exception(exception);
}

fn illegal_constructor(
    scope: &mut v8::PinScope<'_, '_>,
    _: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    throw(scope, Error::Type("Illegal constructor"));
}

fn collect_garbage(
    scope: &mut v8::PinScope<'_, '_>,
    _: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    scope.request_garbage_collection_for_testing(v8::GarbageCollectionType::Full);
}

fn define(
    scope: &v8::PinScope<'_, '_>,
    object: v8::Local<v8::Object>,
    key: v8::Local<v8::Name>,
    value: v8::Local<v8::Value>,
    writable: bool,
    enumerable: bool,
    configurable: bool,
) {
    let mut descriptor = v8::PropertyDescriptor::new_from_value_writable(value, writable);
    descriptor.set_enumerable(enumerable);
    descriptor.set_configurable(configurable);
    assert_eq!(object.define_property(scope, key, &descriptor), Some(true));
}

fn install_interface(
    scope: &mut v8::PinScope<'_, '_>,
    interface: Interface,
    parent: Option<Interface>,
) {
    let state = state(scope);
    let prototype = v8::Object::new(scope);
    let constructor = v8::Function::builder(illegal_constructor)
        .build(scope)
        .expect("interface constructor");
    let interface_name = v8::String::new(scope, name(interface)).expect("interface name");
    constructor.set_name(interface_name);
    if let Some(parent) = parent {
        let state = state.borrow();
        let p = v8::Local::new(scope, &state.prototypes[name(parent)]);
        assert_eq!(prototype.set_prototype(scope, p.into()), Some(true));
        let c = v8::Local::new(scope, &state.constructors[name(parent)]);
        assert_eq!(constructor.set_prototype(scope, c.into()), Some(true));
    }
    let key = v8::String::new(scope, "prototype").expect("prototype key");
    define(
        scope,
        constructor.into(),
        key.into(),
        prototype.into(),
        false,
        false,
        false,
    );
    let key = v8::String::new(scope, "constructor").expect("constructor key");
    define(
        scope,
        prototype,
        key.into(),
        constructor.into(),
        true,
        false,
        true,
    );
    define(
        scope,
        prototype,
        v8::Symbol::get_to_string_tag(scope).into(),
        interface_name.into(),
        false,
        false,
        true,
    );
    define(
        scope,
        scope.get_current_context().global(scope),
        interface_name.into(),
        constructor.into(),
        true,
        false,
        true,
    );
    state.borrow_mut().prototypes.insert(
        name(interface).to_owned(),
        v8::Global::new(scope, prototype),
    );
    state.borrow_mut().constructors.insert(
        name(interface).to_owned(),
        v8::Global::new(scope, constructor),
    );
}

fn install_member(
    scope: &v8::PinScope<'_, '_>,
    interface: Interface,
    name_: &str,
    getter: bool,
    function: v8::Local<v8::Function>,
) {
    let function_name = if getter {
        format!("get {name_}")
    } else {
        name_.to_owned()
    };
    function.set_name(v8::String::new(scope, &function_name).expect("function name"));
    let prototype = v8::Local::new(scope, &state(scope).borrow().prototypes[name(interface)]);
    let key = v8::String::new(scope, name_).expect("member name");
    if getter {
        let mut descriptor =
            v8::PropertyDescriptor::new_from_get_set(function.into(), v8::undefined(scope).into());
        descriptor.set_enumerable(true);
        descriptor.set_configurable(true);
        assert_eq!(
            prototype.define_property(scope, key.into(), &descriptor),
            Some(true)
        );
    } else {
        define(
            scope,
            prototype,
            key.into(),
            function.into(),
            true,
            true,
            true,
        );
    }
}

include!(concat!(env!("OUT_DIR"), "/v8.rs"));

impl Worker {
    pub fn new() -> Result<Self, String> {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let platform = v8::new_default_platform(0, false).make_shared();
            v8::V8::set_flags_from_string("--expose-gc");
            v8::V8::initialize_platform(platform);
            v8::V8::initialize();
        });
        let mut isolate = v8::Isolate::new(Default::default());
        let job_witness = Rc::new(Cell::new(0));
        isolate.set_slot(job_witness.clone());
        Ok(Self {
            isolate,
            job_witness,
        })
    }

    pub fn run_page(&mut self, script: &str) -> Result<PageOutput, String> {
        let before = self.job_witness.get();
        let queue = v8::MicrotaskQueue::new(&mut self.isolate, v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope, &mut self.isolate);
        let context = v8::Context::new(scope, Default::default());
        let default_queue = context.get_microtask_queue().expect("isolate queue");
        context.set_microtask_queue(queue.as_ref());
        let state = Rc::new(RefCell::new(State {
            dom: Dom::new(),
            prototypes: HashMap::new(),
            constructors: HashMap::new(),
            wrappers: HashMap::new(),
        }));
        context.set_slot(state.clone());
        let result = {
            let scope = &mut v8::ContextScope::new(scope, context);
            execute_page(scope, &state, script)
        };
        // Release host handles and the context slot's weak finalizer before teardown GC.
        context.clear_all_slots();
        context.set_microtask_queue(default_queue);
        scope.clear_kept_objects();
        let json = result?;
        let state = state.borrow();
        Ok(PageOutput {
            results: serde_json::from_str(&json).map_err(|e| e.to_string())?,
            dom_attribute: state
                .dom
                .document
                .element_attribute(state.dom.element, "data-final")
                .unwrap_or_default()
                .to_owned(),
            mutation_count: state.dom.mutations,
            job_callbacks: self.job_witness.get() - before,
        })
    }
}

fn execute_page(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Shared,
    script: &str,
) -> Result<String, String> {
    let context = scope.get_current_context();
    install(scope);
    let function = v8::Function::builder(checkpoint)
        .build(scope)
        .expect("checkpoint hook");
    let key = v8::String::new(scope, "checkpoint").expect("hook name");
    define(
        scope,
        context.global(scope),
        key.into(),
        function.into(),
        false,
        false,
        false,
    );
    let function = v8::Function::builder(mark_job)
        .build(scope)
        .expect("job witness hook");
    let key = v8::String::new(scope, "markJob").expect("hook name");
    define(
        scope,
        context.global(scope),
        key.into(),
        function.into(),
        false,
        false,
        false,
    );
    // This embedder hook is a test harness facility, outside the DOM IDL.
    let gc = v8::Function::builder(collect_garbage)
        .build(scope)
        .expect("GC hook");
    let key = v8::String::new(scope, "collectGarbage").expect("GC hook name");
    define(
        scope,
        context.global(scope),
        key.into(),
        gc.into(),
        false,
        false,
        false,
    );
    let element = state.borrow().dom.element;
    for (name_, index) in [("element", element), ("documentNode", 0)] {
        let key = v8::String::new(scope, name_).expect("fixture name");
        let object = wrap(scope, index);
        define(
            scope,
            context.global(scope),
            key.into(),
            object.into(),
            true,
            false,
            true,
        );
    }
    {
        v8::tc_scope!(let scope, scope);
        let source = v8::String::new(scope, script).expect("contract source");
        v8::Script::compile(scope, source, None)
            .and_then(|s| s.run(scope))
            .and_then(|v| v.to_string(scope))
            .map(|s| s.to_rust_string_lossy(scope))
            .ok_or_else(|| {
                scope.exception().map_or_else(
                    || "script evaluation failed".to_owned(),
                    |e| e.to_rust_string_lossy(scope),
                )
            })
    }
}
