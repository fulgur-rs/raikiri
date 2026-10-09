//! Differential cascade checker.
//!
//! Prints a canonical text form of cascade results so two builds of
//! `raikiri-style` — typically a pull request's merge-base and its head — can
//! be compared on the same inputs. `scripts/cascade-diff.sh` builds this tool
//! against both trees and reports every case whose output differs; a change
//! that preserves cascade behavior is expected to report none.
//!
//! ```text
//! raikiri-cascade-diff dump [--seeds START..END] [--html-list FILE]
//! raikiri-cascade-diff show CASE
//! raikiri-cascade-diff describe SEED
//! ```
//!
//! `dump` prints one `CASE<TAB>HASH<TAB>STATUS` line per case, where `STATUS`
//! is `ok`, `error` when the case produced only an error message, or `panic`
//! followed by a fourth column with the first line of the panic message. The
//! case id is written and flushed before the case runs, so a crash or a hang
//! leaves it on an unfinished last line. `show` prints the full canonical text
//! of one case so the two builds can be diffed field by field. `describe`
//! prints the document and stylesheets a seed generates, to turn a reported
//! case into a unit test.
//!
//! Case ids are `gen:SEED:print`, `gen:SEED:screen`, `gen:SEED:first-line`
//! and `html:PATH`. A case that panics is recorded with its panic message
//! instead of a cascade result, so a new panic is reported as a difference.

mod canon;
mod doc;
mod dump;
mod generate;

use std::io::{BufWriter, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::ExitCode;

use raikiri_style::{
    CascadeResult, MediaContext, PseudoElem, RuleTree, StyleDom, StyleNode, StyleNodeId,
    StyleNodeKind,
};

/// Media variants every generated case is cascaded under.
const MEDIA: [&str; 3] = ["print", "screen", "first-line"];

/// Prefixes of a case text that holds an error message instead of a cascade
/// result.
const ERROR_PREFIXES: [&str; 4] = ["BAD CASE: ", "IO-ERROR: ", "PARSE-ERROR: ", "ERR: "];

const USAGE: &str = "usage: raikiri-cascade-diff dump [--seeds START..END] [--html-list FILE]
       raikiri-cascade-diff show CASE
       raikiri-cascade-diff describe SEED";

// cov:ignore: process entry point; the logic lives in `run`, which the unit tests drive with an in-memory writer
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Panics are reported as case output; keep the default hook from also
    // writing them to stderr.
    std::panic::set_hook(Box::new(|_| {}));
    let mut out = BufWriter::new(std::io::stdout().lock());
    let status = run(&args, &mut out);
    if out.flush().is_err() {
        return ExitCode::FAILURE;
    }
    status
}

/// Runs one command, writing its output to `out`.
fn run(args: &[String], out: &mut dyn Write) -> ExitCode {
    let written = match args.first().map(String::as_str) {
        Some("dump") => return dump(&args[1..], out),
        Some("show") if args.len() == 2 => out.write_all(render(&args[1]).as_bytes()),
        Some("describe") if args.len() == 2 => match args[1].parse::<u64>() {
            Ok(seed) => out.write_all(generate::describe(&generate::generate(seed)).as_bytes()),
            Err(_) => return usage(),
        },
        _ => return usage(),
    };
    if written.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn dump(args: &[String], out: &mut dyn Write) -> ExitCode {
    let mut seeds = 0..0u64;
    let mut html = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match (arg.as_str(), iter.next()) {
            ("--seeds", Some(range)) => match parse_range(range) {
                Some(parsed) => seeds = parsed,
                None => return usage(),
            },
            ("--html-list", Some(path)) => match std::fs::read_to_string(path) {
                Ok(list) => html.extend(list.lines().filter(|l| !l.is_empty()).map(str::to_owned)),
                Err(error) => {
                    eprintln!("cannot read {path}: {error}");
                    return ExitCode::from(2);
                }
            },
            _ => return usage(),
        }
    }
    let cases = seeds
        .flat_map(|seed| MEDIA.iter().map(move |media| format!("gen:{seed}:{media}")))
        .chain(html.into_iter().map(|path| format!("html:{path}")));
    for case in cases {
        if write_case_line(out, &case).is_err() {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn write_case_line(out: &mut dyn Write, case: &str) -> std::io::Result<()> {
    // The id goes out before the case runs, so a crash or a hang leaves it on
    // an unfinished last line that names the case.
    write!(out, "{case}\t")?;
    out.flush()?;
    let columns = case_columns(&render(case));
    writeln!(out, "{columns}")?;
    out.flush()
}

/// The `HASH<TAB>STATUS[<TAB>MESSAGE]` columns for a case's rendered text.
fn case_columns(text: &str) -> String {
    let hash = dump::fnv1a(text);
    // A panic either replaced the whole case or ended one of its sections.
    let panic = text.strip_prefix("PANIC: ").or_else(|| {
        let line = text.find("\nPANIC in ")?;
        Some(&text[line + "\nPANIC in ".len()..])
    });
    if let Some(message) = panic {
        let message = message.lines().next().unwrap_or("").replace('\t', " ");
        format!("{hash:016x}\tpanic\t{message}")
    } else if ERROR_PREFIXES.iter().any(|prefix| text.starts_with(prefix)) {
        format!("{hash:016x}\terror")
    } else {
        format!("{hash:016x}\tok")
    }
}

fn parse_range(text: &str) -> Option<std::ops::Range<u64>> {
    let (start, end) = text.split_once("..")?;
    Some(start.parse().ok()?..end.parse().ok()?)
}

/// The canonical text of one case, or a description of why it has none.
fn render(case: &str) -> String {
    guarded(|| render_unguarded(case))
}

/// Runs `f`, turning a panic into a `PANIC: <message>` line.
fn guarded(f: impl FnOnce() -> String) -> String {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(text) => text,
        Err(payload) => format!("PANIC: {}\n", panic_message(&*payload)),
    }
}

/// The message a panic was raised with.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("<non-string panic payload>")
}

fn render_unguarded(case: &str) -> String {
    let bad_case = || format!("BAD CASE: {case}\n");
    if let Some(rest) = case.strip_prefix("gen:") {
        let Some((seed, media)) = rest.split_once(':') else {
            return bad_case();
        };
        let Ok(seed) = seed.parse::<u64>() else {
            return bad_case();
        };
        let generated = generate::generate(seed);
        let mut tree = RuleTree::empty();
        for (source, origin) in &generated.sheets {
            tree.add_stylesheet(source, *origin);
        }
        let context = match media {
            "print" | "first-line" => MediaContext::print(),
            "screen" => MediaContext::screen(),
            _ => return bad_case(),
        };
        let inputs = dump::Inputs {
            dom: &generated.doc,
            tree: &tree,
            media: &context,
            custom_names: &generate::CUSTOM_NAMES,
        };
        let mut out = String::new();
        if media == "first-line" {
            let root = generated.first_line_root.map_or_else(
                || first_line_root(&generated.doc),
                |id| StyleNodeId::new(id as u64),
            );
            first_line_into(&mut out, &inputs, root);
        } else {
            cascade_into(&mut out, &inputs);
        }
        out
    } else if let Some(path) = case.strip_prefix("html:") {
        match std::fs::read(path) {
            Ok(bytes) => html_into(&bytes),
            Err(error) => format!("IO-ERROR: {error}\n"),
        }
    } else {
        bad_case()
    }
}

fn html_into(bytes: &[u8]) -> String {
    let options = raikiri_html::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let mut out = String::new();
    match raikiri_html::parse(bytes, &options) {
        Ok(document) => {
            let tree = raikiri_html::build_rule_tree(&document);
            let media = MediaContext::print();
            let source = String::from_utf8_lossy(bytes);
            let names = dump::custom_property_names(&source);
            let inputs = dump::Inputs {
                dom: &document.dom,
                tree: &tree,
                media: &media,
                custom_names: &names,
            };
            // Layout styles a block's first line through the dedicated entry
            // point, which only takes a block of inline content; the first
            // few blocks with a `::first-line` style give it a chance to find
            // one.
            let origins = cascade_into(&mut out, &inputs)
                .map(|result| first_line_origins(&result))
                .unwrap_or_default();
            for root in origins.into_iter().take(3) {
                match raikiri_style::cascade_with_first_line(&document.dom, &tree, &media, root) {
                    Ok(cascade) => dump::first_line_styles(&mut out, cascade.first_line.as_ref()),
                    Err(error) => out.push_str(&format!("first_line: ERR: {error:?}\n")),
                }
            }
        }
        // cov:ignore: parsing only fails on input-size limits or reader I/O errors, and the tool passes an in-memory byte slice
        Err(error) => out.push_str(&format!("PARSE-ERROR: {error:?}\n")),
    }
    out
}

fn cascade_into<D: StyleDom>(
    out: &mut String,
    inputs: &dump::Inputs<'_, D>,
) -> Option<CascadeResult> {
    match raikiri_style::cascade_with_media_context(inputs.dom, inputs.tree, inputs.media) {
        Ok(result) => {
            dump::cascade_result(out, inputs, &result);
            Some(result)
        }
        // cov:ignore: the cascade documents that it currently always returns Ok; the arm keeps a future error visible as case output
        Err(error) => {
            out.push_str(&format!("ERR: {error:?}\n"));
            None
        }
    }
}

fn first_line_into<D: StyleDom>(out: &mut String, inputs: &dump::Inputs<'_, D>, root: StyleNodeId) {
    match raikiri_style::cascade_with_first_line(inputs.dom, inputs.tree, inputs.media, root) {
        Ok(cascade) => {
            dump::cascade_result(out, inputs, &cascade.normal);
            dump::first_line_styles(out, cascade.first_line.as_ref());
        }
        Err(error) => out.push_str(&format!("ERR: {error:?}\n")),
    }
}

/// The elements that have a `::first-line` style, in node order.
fn first_line_origins(result: &CascadeResult) -> Vec<StyleNodeId> {
    let mut origins: Vec<StyleNodeId> = result
        .pseudo
        .keys()
        .filter(|(_, kind)| *kind == PseudoElem::FirstLine)
        .map(|(id, _)| *id)
        .collect();
    origins.sort_by_key(|id| id.0);
    origins
}

/// The node a generated case without a designated block runs the
/// `::first-line` entry point for: the first in-document element below the
/// root element, or the root element itself.
fn first_line_root<D: StyleDom>(dom: &D) -> StyleNodeId {
    let mut stack: Vec<StyleNodeId> = dom.child_ids(dom.root_id()).collect();
    stack.reverse();
    let mut fallback = None;
    while let Some(id) = stack.pop() {
        let Some(node) = dom.node(id) else { continue };
        if !node.is_in_document() || node.kind() != StyleNodeKind::Element {
            continue;
        }
        if fallback.is_some() {
            return id;
        }
        fallback = Some(id);
        let children: Vec<StyleNodeId> = dom.child_ids(id).collect();
        stack.extend(children.into_iter().rev());
    }
    fallback.unwrap_or_else(|| dom.root_id())
}

#[cfg(test)]
mod tests;
