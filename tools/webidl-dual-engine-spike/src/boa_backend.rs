use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use boa_engine::job::{Job, JobExecutor, SimpleJobExecutor};
use boa_engine::native_function::NativeFunctionPointer;
use boa_engine::object::{FunctionObjectBuilder, JsObject};
use boa_engine::property::PropertyDescriptor;
use boa_engine::{
    Context, Finalize, JsData, JsNativeError, JsResult, JsString, JsSymbol, JsValue,
    NativeFunction, Source, Trace,
};

use crate::shared::{BindingCall, Dom, DomError, Interface, NativeValue, PageOutput};

#[derive(Debug, Trace, Finalize, JsData)]
struct Handle {
    #[unsafe_ignore_trace]
    index: usize,
}

struct State {
    dom: Dom,
    prototypes: HashMap<String, JsObject>,
    constructors: HashMap<String, JsObject>,
    wrappers: HashMap<usize, JsObject>,
}

type Shared = Rc<RefCell<State>>;

#[cfg(test)]
#[path = "boa_backend/tests.rs"]
mod tests;

#[derive(Trace, Finalize, JsData)]
struct RealmState {
    #[unsafe_ignore_trace]
    shared: Shared,
}

#[derive(Default)]
struct PageJobs(RefCell<Rc<SimpleJobExecutor>>);

impl PageJobs {
    fn reset(&self) {
        *self.0.borrow_mut() = Rc::new(SimpleJobExecutor::new());
    }
}

impl JobExecutor for PageJobs {
    fn enqueue_job(self: Rc<Self>, job: Job, context: &mut Context) {
        self.0.borrow().clone().enqueue_job(job, context);
    }

    fn run_jobs(self: Rc<Self>, context: &mut Context) -> JsResult<()> {
        let jobs = self.0.borrow().clone();
        jobs.run_jobs(context)
    }
}

pub(crate) struct Worker {
    context: Context,
    jobs: Rc<PageJobs>,
    job_witness: Rc<Cell<usize>>,
}

fn state(context: &Context) -> Shared {
    context
        .realm()
        .host_defined()
        .get::<RealmState>()
        .expect("installed state")
        .shared
        .clone()
}

fn checkpoint(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    context.run_jobs()?;
    Ok(JsValue::undefined())
}

fn mark_job(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let witness = context
        .get_data::<Rc<Cell<usize>>>()
        .expect("worker witness");
    witness.set(witness.get() + 1);
    Ok(JsValue::undefined())
}
fn type_error(message: &'static str) -> boa_engine::JsError {
    JsNativeError::typ().with_message(message).into()
}
fn name(interface: Interface) -> &'static str {
    match interface {
        Interface::Node => "Node",
        Interface::Element => "Element",
    }
}

fn handle(value: &JsValue, interface: Interface, context: &Context) -> JsResult<usize> {
    let index = value
        .as_object()
        .and_then(|o| o.downcast_ref::<Handle>().map(|h| h.index))
        .ok_or_else(|| type_error("Illegal invocation"))?;
    if !state(context).borrow().dom.supports(index, interface) {
        return Err(type_error("Illegal invocation"));
    }
    Ok(index)
}

fn wrap(context: &Context, index: usize) -> JsObject {
    let state = state(context);
    let mut state = state.borrow_mut();
    if let Some(object) = state.wrappers.get(&index) {
        return object.clone();
    }
    let prototype = state.prototypes[name(state.dom.interface(index))].clone();
    let object = JsObject::from_proto_and_data(Some(prototype), Handle { index });
    state.wrappers.insert(index, object.clone());
    object
}

struct Call<'a> {
    this: &'a JsValue,
    args: &'a [JsValue],
    context: &'a mut Context,
}

impl BindingCall for Call<'_> {
    type Value = JsValue;
    type Error = boa_engine::JsError;
    fn receiver(&mut self, interface: Interface) -> JsResult<usize> {
        handle(self.this, interface, self.context)
    }
    fn require(&mut self, count: usize) -> JsResult<()> {
        if self.args.len() < count {
            Err(type_error("required argument missing"))
        } else {
            Ok(())
        }
    }
    fn dom_string(&mut self, index: usize) -> JsResult<String> {
        // Preserve JS exceptions and avoid escaping or silently replacing UTF-16 surrogates.
        self.args[index]
            .to_string(self.context)?
            .to_std_string()
            .map_err(|_| type_error("prototype DOM storage requires valid Unicode"))
    }
    fn nullable_node(&mut self, index: usize) -> JsResult<Option<usize>> {
        let value = &self.args[index];
        if value.is_null() || value.is_undefined() {
            Ok(None)
        } else {
            handle(value, Interface::Node, self.context).map(Some)
        }
    }
    fn with_dom<R>(&mut self, f: impl FnOnce(&mut Dom) -> R) -> R {
        f(&mut state(self.context).borrow_mut().dom)
    }
    fn native_error(&mut self, error: DomError) -> Self::Error {
        let object = JsNativeError::error()
            .with_message(error.0)
            .into_opaque(self.context);
        object
            .define_property_or_throw(
                JsString::from("name"),
                PropertyDescriptor::builder()
                    .value(JsString::from("InvalidCharacterError"))
                    .writable(true)
                    .enumerable(false)
                    .configurable(true)
                    .build(),
                self.context,
            )
            .expect("fresh error own property");
        boa_engine::JsError::from_opaque(object.into())
    }
    fn output(&mut self, value: NativeValue) -> JsResult<JsValue> {
        Ok(match value {
            NativeValue::Number(v) => JsValue::from(v),
            NativeValue::Boolean(v) => JsValue::from(v),
            NativeValue::String(v) => v.map_or_else(JsValue::null, |v| JsString::from(v).into()),
            NativeValue::Node(v) => v.map_or_else(JsValue::null, |v| wrap(self.context, v).into()),
            NativeValue::Undefined(()) => JsValue::undefined(),
        })
    }
}

fn illegal_constructor(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Err(type_error("Illegal constructor"))
}

fn collect_garbage(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    boa_gc::force_collect();
    Ok(JsValue::undefined())
}

fn install_interface(
    context: &mut Context,
    interface: Interface,
    parent: Option<Interface>,
) -> JsResult<()> {
    let state = state(context);
    let parent_prototype = parent.map(|p| state.borrow().prototypes[name(p)].clone());
    let prototype = JsObject::with_object_proto(context.intrinsics());
    if let Some(parent) = parent_prototype {
        assert!(prototype.set_prototype(Some(parent)));
    }
    let constructor = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(illegal_constructor),
    )
    .name(JsString::from(name(interface)))
    .length(0)
    .constructor(true)
    .build();
    if let Some(parent) = parent {
        assert!(constructor.set_prototype(Some(state.borrow().constructors[name(parent)].clone())));
    }
    constructor.define_property_or_throw(
        JsString::from("prototype"),
        PropertyDescriptor::builder()
            .value(prototype.clone())
            .writable(false)
            .enumerable(false)
            .configurable(false)
            .build(),
        context,
    )?;
    prototype.define_property_or_throw(
        JsString::from("constructor"),
        PropertyDescriptor::builder()
            .value(constructor.clone())
            .writable(true)
            .enumerable(false)
            .configurable(true)
            .build(),
        context,
    )?;
    prototype.define_property_or_throw(
        JsSymbol::to_string_tag(),
        PropertyDescriptor::builder()
            .value(JsString::from(name(interface)))
            .writable(false)
            .enumerable(false)
            .configurable(true)
            .build(),
        context,
    )?;
    context.global_object().define_property_or_throw(
        JsString::from(name(interface)),
        PropertyDescriptor::builder()
            .value(constructor.clone())
            .writable(true)
            .enumerable(false)
            .configurable(true)
            .build(),
        context,
    )?;
    state
        .borrow_mut()
        .prototypes
        .insert(name(interface).to_owned(), prototype);
    state
        .borrow_mut()
        .constructors
        .insert(name(interface).to_owned(), constructor.into());
    Ok(())
}

fn install_member(
    context: &mut Context,
    interface: Interface,
    name_: &str,
    getter: bool,
    length: usize,
    callback: NativeFunctionPointer,
) -> JsResult<()> {
    let function_name = if getter {
        format!("get {name_}")
    } else {
        name_.to_owned()
    };
    let function =
        FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(callback))
            .name(JsString::from(function_name))
            .length(length)
            .constructor(false)
            .build();
    let descriptor = if getter {
        PropertyDescriptor::builder()
            .get(function)
            .enumerable(true)
            .configurable(true)
            .build()
    } else {
        PropertyDescriptor::builder()
            .value(function)
            .writable(true)
            .enumerable(true)
            .configurable(true)
            .build()
    };
    let prototype = state(context).borrow().prototypes[name(interface)].clone();
    prototype.define_property_or_throw(JsString::from(name_), descriptor, context)?;
    Ok(())
}

include!(concat!(env!("OUT_DIR"), "/boa.rs"));

impl Worker {
    pub fn new() -> Result<Self, String> {
        let jobs = Rc::new(PageJobs::default());
        let job_witness = Rc::new(Cell::new(0));
        let mut context = Context::builder()
            .job_executor(jobs.clone())
            .build()
            .map_err(|e| e.to_string())?;
        context.insert_data(job_witness.clone());
        Ok(Self {
            context,
            jobs,
            job_witness,
        })
    }

    pub fn run_page(&mut self, script: &str) -> Result<PageOutput, String> {
        self.jobs.reset();
        let realm = self.context.create_realm().map_err(|e| e.to_string())?;
        let previous = self.context.enter_realm(realm);
        let state = Rc::new(RefCell::new(State {
            dom: Dom::new(),
            prototypes: HashMap::new(),
            constructors: HashMap::new(),
            wrappers: HashMap::new(),
        }));
        self.context.realm().host_defined_mut().insert(RealmState {
            shared: state.clone(),
        });
        let before = self.job_witness.get();
        let result = execute_page(&mut self.context, &state, script).map(
            |(results, dom_attribute, mutation_count)| PageOutput {
                results,
                dom_attribute,
                mutation_count,
                job_callbacks: self.job_witness.get() - before,
            },
        );
        // Break host roots and discard queued jobs on both success and JS error paths.
        self.context
            .realm()
            .host_defined_mut()
            .remove::<RealmState>();
        self.context.enter_realm(previous);
        self.jobs.reset();
        self.context.clear_kept_objects();
        result
    }
}

fn execute_page(
    context: &mut Context,
    state: &Shared,
    script: &str,
) -> Result<(serde_json::Value, String, usize), String> {
    install(context).map_err(|e| e.to_string())?;
    for (name, function) in [
        ("checkpoint", checkpoint as NativeFunctionPointer),
        ("markJob", mark_job),
    ] {
        context
            .register_global_builtin_callable(
                JsString::from(name),
                0,
                NativeFunction::from_fn_ptr(function),
            )
            .map_err(|e| e.to_string())?;
    }
    // This embedder hook is a test harness facility, outside the DOM IDL.
    let gc = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(collect_garbage),
    )
    .name(JsString::from("collectGarbage"))
    .build();
    context
        .global_object()
        .define_property_or_throw(
            JsString::from("collectGarbage"),
            PropertyDescriptor::builder().value(gc).build(),
            context,
        )
        .map_err(|e| e.to_string())?;
    let element = state.borrow().dom.element;
    for (name_, index) in [("element", element), ("documentNode", 0)] {
        let wrapper = wrap(context, index);
        context
            .global_object()
            .define_property_or_throw(
                JsString::from(name_),
                PropertyDescriptor::builder()
                    .value(wrapper)
                    .writable(true)
                    .configurable(true)
                    .build(),
                context,
            )
            .map_err(|e| e.to_string())?;
    }
    let result = context
        .eval(Source::from_bytes(script))
        .map_err(|e| e.to_string())?;
    let json = result
        .to_string(context)
        .map_err(|e| e.to_string())?
        .to_std_string()
        .map_err(|e| e.to_string())?;
    let state = state.borrow();
    let attr = state
        .dom
        .document
        .element_attribute(state.dom.element, "data-final")
        .unwrap_or_default()
        .to_owned();
    Ok((
        serde_json::from_str(&json).map_err(|e| e.to_string())?,
        attr,
        state.dom.mutations,
    ))
}
