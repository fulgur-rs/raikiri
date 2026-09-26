//! Script execution boundary between a document driver and a parser.
//!
//! `ScriptExecutor` is the seam a streaming HTML parser (raikiri-html) would
//! call into to run a `<script>` element the moment it closes, without that
//! parser depending on raikiri-js directly (raikiri-html cannot depend on
//! raikiri-js; see this crate's own role as the shared trait layer between
//! them). Nothing in raikiri-html calls this yet: today, only
//! `raikiri_js::runtime::DomRuntime::run_document` implements it, and it
//! calls its own implementation once per document, after the whole document
//! has already been parsed, for every `<script>` element in tree order. A
//! parser that adopted this seam later would call it as each script element
//! closes instead, and could use the `Block` outcome to pause itself while a
//! blocking script fetch is outstanding.

use crate::dom::NodeId;

/// Whether the caller should keep going after
/// [`ScriptExecutor::execute_script`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)] // cov:ignore: no coverage region either; see below
//
// cov:ignore: a plain enum declaration with no method body of its own gets
// no coverage region under source-based instrumentation, even though
// `script/tests.rs` (this crate's own test binary) constructs both
// variants, compares, and debug-formats them -- there is no generated code
// at these exact lines for an execution count to attach to.
pub enum ScriptExecution {
    /// Keep going (parsing, or whatever else the caller was doing).
    Continue,
    /// Pause: a blocking script is still being fetched. No implementation in
    /// this workspace produces this today (see this module's own doc
    /// comment) -- it exists for a streaming parser's sake, which needs to
    /// suspend tokenizing while such a script is outstanding.
    Block,
}

/// Runs one `<script>` element.
//
// cov:ignore: same reasoning as `ScriptExecution` above -- a trait
// declaration with no default method body of its own gets no coverage
// region either, even though `script/tests.rs` implements it and calls
// `execute_script` both directly and through `dyn ScriptExecutor`.
pub trait ScriptExecutor {
    /// Run the script element `element` (fetching it first if it is an
    /// external script, or reading its inline text otherwise); a `type`
    /// attribute this executor does not recognize as a classic script is
    /// simply not run. `Block` asks a streaming parser to pause.
    fn execute_script(&mut self, element: NodeId) -> ScriptExecution;
}

#[cfg(test)] // cov:ignore: no coverage region of its own; see the comments above
mod tests;
