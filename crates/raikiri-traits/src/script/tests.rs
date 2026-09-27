use super::{ScriptExecution, ScriptExecutor};
use crate::dom::NodeId;

/// A minimal `ScriptExecutor`: records every element it was asked to run and
/// always returns a fixed outcome, enough to exercise the trait's calling
/// convention without a real DOM or JS engine behind it.
struct Recorder {
    calls: Vec<NodeId>,
    next: ScriptExecution,
}

impl ScriptExecutor for Recorder {
    fn execute_script(&mut self, element: NodeId) -> ScriptExecution {
        self.calls.push(element);
        self.next
    }
}

#[test]
fn script_execution_variants_are_distinct_and_derive_debug_clone_copy_eq() {
    assert_eq!(ScriptExecution::Continue, ScriptExecution::Continue);
    assert_ne!(ScriptExecution::Continue, ScriptExecution::Block);
    let copied = ScriptExecution::Block;
    assert_eq!(copied, ScriptExecution::Block);
    assert_eq!(format!("{:?}", ScriptExecution::Continue), "Continue");
    assert_eq!(format!("{:?}", ScriptExecution::Block), "Block");
}

#[test]
fn script_executor_is_callable_directly_and_through_a_trait_object() {
    let element = NodeId::new(7);

    let mut recorder = Recorder {
        calls: Vec::new(),
        next: ScriptExecution::Block,
    };
    assert_eq!(recorder.execute_script(element), ScriptExecution::Block);
    assert_eq!(recorder.calls, vec![element]);

    // A streaming parser would hold this behind a trait object rather than
    // a concrete type (see this module's own doc comment); confirm the
    // trait stays object-safe and dispatches correctly through one.
    let mut boxed: Box<dyn ScriptExecutor> = Box::new(Recorder {
        calls: Vec::new(),
        next: ScriptExecution::Continue,
    });
    assert_eq!(boxed.execute_script(element), ScriptExecution::Continue);
}
