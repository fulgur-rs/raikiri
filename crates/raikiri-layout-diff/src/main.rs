//! Differential layout checker.
//!
//! Prints a canonical text form of layout results so two builds of
//! `raikiri-dom` and `raikiri-html` — typically a pull request's merge-base
//! and its head — can be compared on the same inputs, down to the bits of
//! every `f32`. `scripts/layout-diff.sh` builds this tool against both trees
//! and reports every case whose output differs; a change that preserves
//! layout is expected to report none. It is the layout counterpart of
//! `raikiri-cascade-diff`, and a pass/fail conformance run cannot stand in for
//! it: a layout that moves a box by a hundredth of a pixel passes the same
//! tests.
//!
//! ```text
//! raikiri-layout-diff dump [--font FILE] [--seeds START..END] [--html-list FILE] [--html-root DIR]
//! raikiri-layout-diff show [--font FILE] [--html-root DIR] CASE
//! raikiri-layout-diff describe SEED
//! raikiri-layout-diff compiler
//! ```
//!
//! `dump` prints one `CASE<TAB>HASH<TAB>STATUS` line per case, where `STATUS`
//! is `ok`, `error` when the case produced only an error message, or `panic`
//! followed by a fourth column with the first line of the panic message. The
//! case id is written and flushed before the case runs, so a crash or a hang
//! leaves it on an unfinished last line. `show` prints the full canonical text
//! of one case so the two builds can be diffed field by field. `describe`
//! prints the document a seed generates, to turn a reported case into a unit
//! test. `compiler` prints the `-vV` output of the compiler cargo built the
//! tool with, which the script keys its cached builds by.
//!
//! Case ids are `gen:SEED` and `html:PATH`. A case that panics is recorded
//! with its panic message instead of a layout, so a new panic is reported as a
//! difference.
//!
//! With `--html-root`, an HTML case under that directory is parsed at the URL
//! `file:///` plus its path relative to the directory, and its linked
//! stylesheets, fonts and images are read from the directory, so root-relative
//! links resolve as they do on a server rooted there.
//!
//! With `--font`, the cases are laid out in that one font, registered as
//! `Ahem`, plus the faces a document declares itself, and the installed fonts
//! play no part, so the result does not depend on the machine. Without it the
//! installed fonts are used.
//!
//! The canonical text is described in the module that writes it, and the
//! README of this crate says how to read a report and how to check the tool.

mod dump;
mod files;
mod generate;
mod node_layout;

use std::cell::Cell;
use std::io::{BufWriter, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use raikiri_html::{
    FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, MediaContext, MediaType,
    PageDefaults, RenderResources, parse_html_with_resources,
};
use raikiri_traits::{PageBox, RenderError};

/// The size, in CSS px, of the page every case is laid out on unless the
/// document sets its own, and of the viewport its media queries see.
const PAGE_SIZE: (u32, u32) = (800, 600);

/// Prefixes of a case text that holds an error message instead of a layout.
const ERROR_PREFIXES: [&str; 4] = ["BAD CASE: ", "IO-ERROR: ", "PARSE-ERROR: ", "ERR: "];

const USAGE: &str = "usage: raikiri-layout-diff dump [--font FILE] [--seeds START..END] [--html-list FILE] [--html-root DIR]
       raikiri-layout-diff show [--font FILE] [--html-root DIR] CASE
       raikiri-layout-diff describe SEED
       raikiri-layout-diff compiler";

/// The `-vV` output of the compiler cargo built this binary with, as the
/// build script recorded it.
const COMPILER: &str = include_str!(concat!(env!("OUT_DIR"), "/compiler"));

// cov:ignore: process entry point; the logic lives in `run`, which the unit tests drive with an in-memory writer
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // A panic inside a case is reported as the case's output; keep the default
    // hook from also writing it to stderr. A panic anywhere else is a defect
    // of the tool and keeps its message and location.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !in_case() {
            default_hook(info);
        }
    }));
    let mut out = BufWriter::new(std::io::stdout().lock());
    let status = run(&args, &mut out);
    if out.flush().is_err() {
        return ExitCode::FAILURE;
    }
    status
}

/// Runs one command, writing its output to `out`.
fn run(args: &[String], out: &mut dyn Write) -> ExitCode {
    let Some((command, rest)) = args.split_first() else {
        return usage();
    };
    match command.as_str() {
        "dump" => dump(rest, out),
        "show" => show(rest, out),
        "describe" => match rest {
            [seed] => match seed.parse::<u64>() {
                Ok(seed) => finish(out.write_all(generate::generate(seed).as_bytes())),
                Err(_) => usage(),
            },
            _ => usage(),
        },
        "compiler" if rest.is_empty() => finish(out.write_all(COMPILER.as_bytes())),
        _ => usage(),
    }
}

fn finish(written: std::io::Result<()>) -> ExitCode {
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

/// The command line after the command name: the `--flag value` pairs and the
/// other arguments, in order.
struct Arguments<'a> {
    flags: Vec<(&'a str, &'a str)>,
    positional: Vec<&'a str>,
}

impl<'a> Arguments<'a> {
    /// Splits `args`, accepting only the flags in `allowed`; `None` when an
    /// argument starting with `--` is not one of them or lacks its value.
    fn parse(args: &'a [String], allowed: &[&str]) -> Option<Self> {
        let mut parsed = Self {
            flags: Vec::new(),
            positional: Vec::new(),
        };
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if allowed.contains(&arg.as_str()) {
                parsed.flags.push((arg, iter.next()?));
            } else if arg.starts_with("--") {
                return None;
            } else {
                parsed.positional.push(arg);
            }
        }
        Some(parsed)
    }

    /// The values of `flag`, in order.
    fn values<'s>(&'s self, flag: &'s str) -> impl Iterator<Item = &'a str> + 's {
        self.flags
            .iter()
            .filter(move |(name, _)| *name == flag)
            .map(|(_, value)| *value)
    }

    /// The last value of `flag`.
    fn last(&self, flag: &str) -> Option<&'a str> {
        self.values(flag).last()
    }
}

/// What every case is laid out with.
struct Setup {
    /// The font set, when `--font` names one.
    fonts: Option<raikiri_html::RenderFonts>,
    /// The directory linked files are read from, when `--html-root` names one.
    root: Option<PathBuf>,
}

impl Setup {
    /// The setup the `--font` and `--html-root` flags of `arguments` ask for,
    /// or the message that says why it cannot be had.
    fn from_arguments(arguments: &Arguments<'_>) -> Result<Self, String> {
        let fonts = arguments
            .last("--font")
            .map(|path| {
                let bytes =
                    std::fs::read(path).map_err(|error| format!("cannot read {path}: {error}"))?;
                FontCollectionBuilder::new()
                    .font_bytes("Ahem", bytes)
                    .build()
                    .map_err(|error| format!("cannot use {path} as a font: {error}"))
            })
            .transpose()?;
        Ok(Self {
            fonts,
            root: arguments.last("--html-root").map(PathBuf::from),
        })
    }
}

fn dump(args: &[String], out: &mut dyn Write) -> ExitCode {
    let flags = ["--font", "--seeds", "--html-list", "--html-root"];
    let Some(arguments) = Arguments::parse(args, &flags) else {
        return usage();
    };
    if !arguments.positional.is_empty() {
        return usage();
    }
    let mut seeds = 0..0u64;
    if let Some(range) = arguments.last("--seeds") {
        match parse_range(range) {
            Some(parsed) => seeds = parsed,
            None => return usage(),
        }
    }
    let mut html = Vec::new();
    for path in arguments.values("--html-list") {
        match std::fs::read_to_string(path) {
            Ok(list) => html.extend(list.lines().filter(|l| !l.is_empty()).map(str::to_owned)),
            Err(error) => {
                eprintln!("cannot read {path}: {error}");
                return ExitCode::from(2);
            }
        }
    }
    // A tab in a case id would shift the columns of its output line.
    if let Some(path) = html.iter().find(|path| path.contains('\t')) {
        eprintln!("a tab in the path {path:?} is not supported");
        return ExitCode::from(2);
    }
    let setup = match Setup::from_arguments(&arguments) {
        Ok(setup) => setup,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let cases = seeds
        .map(|seed| format!("gen:{seed}"))
        .chain(html.into_iter().map(|path| format!("html:{path}")));
    for case in cases {
        if write_case_line(out, &case, &setup).is_err() {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn show(args: &[String], out: &mut dyn Write) -> ExitCode {
    let Some(arguments) = Arguments::parse(args, &["--font", "--html-root"]) else {
        return usage();
    };
    let [case] = arguments.positional[..] else {
        return usage();
    };
    match Setup::from_arguments(&arguments) {
        Ok(setup) => finish(out.write_all(render(case, &setup).as_bytes())),
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

fn write_case_line(out: &mut dyn Write, case: &str, setup: &Setup) -> std::io::Result<()> {
    // The id goes out before the case runs, so a crash or a hang leaves it on
    // an unfinished last line that names the case.
    write!(out, "{case}\t")?;
    out.flush()?;
    let columns = case_columns(&render(case, setup));
    writeln!(out, "{columns}")?;
    out.flush()
}

/// The `HASH<TAB>STATUS[<TAB>MESSAGE]` columns for a case's rendered text.
fn case_columns(text: &str) -> String {
    let hash = dump::fnv1a(text);
    if let Some(message) = text.strip_prefix("PANIC: ") {
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
fn render(case: &str, setup: &Setup) -> String {
    guarded(|| render_unguarded(case, setup))
}

thread_local! {
    /// Whether this thread is running a case under [`guarded`].
    static IN_CASE: Cell<bool> = const { Cell::new(false) };
}

/// Whether the calling thread is running a case, whose panic is reported as
/// its output.
fn in_case() -> bool {
    IN_CASE.get()
}

/// Runs `f`, turning a panic into a `PANIC: <message>` line.
fn guarded(f: impl FnOnce() -> String) -> String {
    IN_CASE.set(true);
    let result = catch_unwind(AssertUnwindSafe(f));
    IN_CASE.set(false);
    match result {
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

fn render_unguarded(case: &str, setup: &Setup) -> String {
    if let Some(seed) = case.strip_prefix("gen:") {
        match seed.parse::<u64>() {
            Ok(seed) => lay_out(generate::generate(seed).as_bytes(), None, setup),
            Err(_) => format!("BAD CASE: {case}\n"),
        }
    } else if let Some(path) = case.strip_prefix("html:") {
        match std::fs::read(path) {
            Ok(bytes) => {
                let base_url = setup
                    .root
                    .as_deref()
                    .and_then(|root| files::document_url(root, Path::new(path)));
                lay_out(&bytes, base_url, setup)
            }
            Err(error) => format!("IO-ERROR: {error}\n"),
        }
    } else {
        format!("BAD CASE: {case}\n")
    }
}

/// Parses and lays out the HTML document `bytes` and writes its canonical
/// text. `base_url` is the address the document is parsed at.
fn lay_out(bytes: &[u8], base_url: Option<url::Url>, setup: &Setup) -> String {
    let files = setup
        .root
        .as_ref()
        .map(|root| files::RootedFiles { root: root.clone() });
    let mut resources = RenderResources::new();
    if let Some(files) = &files {
        resources = resources.network_provider(files);
    }
    if let Some(base_url) = base_url {
        resources = resources.base_url(base_url);
    }
    if let Some(fonts) = &setup.fonts {
        resources = resources.fonts(fonts.clone());
    }
    let document = match parse_html_with_resources(bytes, &resources) {
        Ok(document) => document,
        Err(error) => return format!("PARSE-ERROR: {error:?}\n"),
    };
    let mut page_box = PageBox::new();
    (page_box.width, page_box.height) = (PAGE_SIZE.0 as f32, PAGE_SIZE.1 as f32);
    let defaults = PageDefaults::builder().page_box(page_box).build();
    let config = LayoutConfig::builder()
        .media_context(MediaContext::with_viewport(
            MediaType::Print,
            PAGE_SIZE.0,
            PAGE_SIZE.1,
        ))
        .build();
    let status = raikiri_html::layout(
        &document,
        defaults,
        config,
        LayoutOptions::new().resources(&resources),
    );
    layout_text(status)
}

/// The canonical text of the layout a call to `raikiri_html::layout`
/// produced, or an error line saying why it produced none.
fn layout_text(status: Result<LayoutStatus, RenderError>) -> String {
    match status {
        Ok(LayoutStatus::Completed(layout)) => {
            let mut out = String::new();
            dump::document(&mut out, &layout);
            out
        }
        Ok(_) => "ERR: the layout did not complete\n".to_owned(),
        Err(error) => format!("ERR: {error:?}\n"),
    }
}

#[cfg(test)]
mod tests;
