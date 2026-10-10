use super::*;
use crate::doc::{GenDoc, GenNode};
use raikiri_style::{Origin, StyleQuirksMode};

fn run_to_string(args: &[&str]) -> (ExitCode, String) {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let mut out = Vec::new();
    let status = run(&args, &mut out);
    (status, String::from_utf8(out).expect("utf-8 output"))
}

/// A writer whose every write fails.
struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("closed"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A writer that records what it holds at each flush.
#[derive(Default)]
struct FlushLog {
    bytes: Vec<u8>,
    flushed: Vec<String>,
}

impl Write for FlushLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flushed
            .push(String::from_utf8_lossy(&self.bytes).into_owned());
        Ok(())
    }
}

/// A uniquely named file in the system temporary directory, removed on drop.
struct TempFile(std::path::PathBuf);

impl TempFile {
    fn new(name: &str, contents: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "raikiri-cascade-diff-{}-{name}",
            std::process::id()
        ));
        std::fs::write(&path, contents).expect("write temporary file");
        Self(path)
    }

    fn path(&self) -> &str {
        self.0.to_str().expect("utf-8 path")
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn dump_prints_one_line_per_case() {
    let (status, out) = run_to_string(&["dump", "--seeds", "0..4"]);
    assert_eq!(status, ExitCode::SUCCESS);
    assert_eq!(out.lines().count(), 4 * MEDIA.len());
    for line in out.lines() {
        let columns: Vec<&str> = line.split('\t').collect();
        assert!(columns[0].starts_with("gen:"), "{line}");
        assert_eq!(columns[1].len(), 16, "{line}");
        match columns[2] {
            "ok" | "error" => assert_eq!(columns.len(), 3, "{line}"),
            "panic" => assert_eq!(columns.len(), 4, "{line}"),
            other => panic!("unexpected status {other}"),
        }
    }
}

#[test]
fn dump_names_a_case_before_running_it() {
    let mut log = FlushLog::default();
    let args = vec!["dump".to_owned(), "--seeds".to_owned(), "0..1".to_owned()];
    assert_eq!(run(&args, &mut log), ExitCode::SUCCESS);
    assert_eq!(log.flushed[0], "gen:0:print\t");
    assert!(log.flushed[1].starts_with("gen:0:print\t"));
    assert!(log.flushed[1].ends_with('\n'));
    assert_eq!(log.flushed[2], format!("{}gen:0:screen\t", log.flushed[1]));
}

#[test]
fn case_columns_name_the_status() {
    let text = "PANIC: a\tb\nsecond line\n";
    assert_eq!(
        case_columns(text),
        format!("{:016x}\tpanic\ta b", dump::fnv1a(text))
    );
    let text = "root_element_index: Some(1)\nPANIC in first letters: a\tb\nmore\n";
    assert_eq!(
        case_columns(text),
        format!("{:016x}\tpanic\tfirst letters: a b", dump::fnv1a(text))
    );
    for text in [
        "ERR: x\n",
        "IO-ERROR: x\n",
        "PARSE-ERROR: x\n",
        "BAD CASE: x\n",
    ] {
        assert_eq!(
            case_columns(text),
            format!("{:016x}\terror", dump::fnv1a(text))
        );
    }
    let text = "root_element_index: Some(1)\n";
    assert_eq!(
        case_columns(text),
        format!("{:016x}\tok", dump::fnv1a(text))
    );
}

#[test]
fn dump_reads_html_cases_from_a_list() {
    let html = TempFile::new(
        "case.html",
        "<!doctype html><style>p { color: red }</style><p>x</p>",
    );
    let list = TempFile::new(
        "list.txt",
        &format!(
            "{}\n\n/nonexistent/raikiri-cascade-diff.html\n",
            html.path()
        ),
    );
    let (status, out) = run_to_string(&["dump", "--html-list", list.path()]);
    assert_eq!(status, ExitCode::SUCCESS);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with(&format!("html:{}\t", html.path())));
    assert!(render(&format!("html:{}", html.path()), None).starts_with("root_element_index:"));
    assert!(render("html:/nonexistent/raikiri-cascade-diff.html", None).starts_with("IO-ERROR:"));
}

#[test]
fn html_cases_under_a_root_read_their_linked_stylesheets() {
    let root = std::env::temp_dir().join(format!(
        "raikiri-cascade-diff-{}-html-root",
        std::process::id()
    ));
    std::fs::create_dir_all(root.join("css/support")).unwrap();
    std::fs::create_dir_all(root.join("css/test")).unwrap();
    std::fs::write(root.join("css/support/root.css"), "p { margin-left: 7px }").unwrap();
    std::fs::write(
        root.join("css/test/local.css"),
        "@import \"rules.txt\"; p { --local: 1; --\\65 sc: 2 }",
    )
    .unwrap();
    std::fs::write(root.join("css/test/rules.txt"), "p { margin-right: 9px }").unwrap();
    let page = root.join("css/test/page.html");
    std::fs::write(
        &page,
        "<!doctype html><link rel=stylesheet href=\"/css/support/root.css\">\
         <link rel=stylesheet href=\"local.css\"><p style=\"--\\61 ttr: 3\">x</p>",
    )
    .unwrap();
    let case = format!("html:{}", page.display());
    let linked = render(&case, Some(&root));
    // A root-relative and a relative link both resolve against the root, and
    // custom properties declared only in a linked stylesheet are printed,
    // effective and local alike, as are those whose names are escaped there
    // or in a style attribute.
    assert!(linked.contains("left: Px(7.0) }"), "{linked}");
    assert!(
        linked.contains(": --attr=\"3\", --esc=\"2\", --local=\"1\""),
        "{linked}"
    );
    assert!(
        linked.contains(" local: --attr=\"3\", --esc=\"2\", --local=\"1\""),
        "{linked}"
    );
    // A file that is not CSS is not imported.
    assert!(!linked.contains("right: Px(9.0)"), "{linked}");
    // Without a root the links are not followed.
    let unlinked = render(&case, None);
    assert!(!unlinked.contains("left: Px(7.0) }"), "{unlinked}");
    // `show` takes the same root.
    let (status, shown) = run_to_string(&["show", "--html-root", root.to_str().unwrap(), &case]);
    assert_eq!(status, ExitCode::SUCCESS);
    assert_eq!(shown, linked);
    // Sharing of custom-property bindings is not printed, only their values.
    assert!(
        !linked.contains("CustomPropertyEnvironment { local: {\"--local\""),
        "{linked}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn invalid_arguments_are_usage_errors() {
    for args in [
        &[][..],
        &["show"][..],
        &["describe", "x"][..],
        &["compiler", "x"][..],
        &["bogus"][..],
        &["dump", "--seeds", "1-2"][..],
        &["dump", "--seeds"][..],
        &["dump", "--bogus", "x"][..],
        &[
            "dump",
            "--html-list",
            "/nonexistent/raikiri-cascade-diff-list.txt",
        ][..],
    ] {
        assert_eq!(run_to_string(args).0, ExitCode::from(2), "{args:?}");
    }
}

#[test]
fn show_and_describe_print_one_case() {
    let (status, out) = run_to_string(&["show", "gen:3:print"]);
    assert_eq!(status, ExitCode::SUCCESS);
    assert_eq!(out, render("gen:3:print", None));
    let (status, out) = run_to_string(&["describe", "3"]);
    assert_eq!(status, ExitCode::SUCCESS);
    assert!(out.contains("/* Author */"));
}

#[test]
fn compiler_prints_the_version_the_build_script_recorded() {
    let (status, out) = run_to_string(&["compiler"]);
    assert_eq!(status, ExitCode::SUCCESS);
    assert_eq!(out, COMPILER);
    assert!(out.starts_with("rustc "), "{out}");
    assert!(out.contains("\nrelease: "), "{out}");
}

#[test]
fn write_failures_are_reported() {
    let args = vec!["dump".to_owned(), "--seeds".to_owned(), "0..1".to_owned()];
    assert_eq!(run(&args, &mut FailingWriter), ExitCode::FAILURE);
    let args = vec!["show".to_owned(), "gen:0:print".to_owned()];
    assert_eq!(run(&args, &mut FailingWriter), ExitCode::FAILURE);
    let args = vec!["compiler".to_owned()];
    assert_eq!(run(&args, &mut FailingWriter), ExitCode::FAILURE);
}

#[test]
fn malformed_case_ids_are_reported() {
    for case in ["gen:3", "gen:x:print", "gen:3:bogus", "other:3"] {
        assert_eq!(render(case, None), format!("BAD CASE: {case}\n"));
    }
}

#[test]
fn panics_become_case_output() {
    assert_eq!(guarded(|| panic!("{}", "boom".to_owned())), "PANIC: boom\n");
    assert_eq!(guarded(|| panic!("static")), "PANIC: static\n");
    assert_eq!(
        guarded(|| std::panic::panic_any(5_u8)),
        "PANIC: <non-string panic payload>\n"
    );
}

/// The `::first-line` entry point's dump for `root` of `doc` under one author
/// stylesheet.
fn first_line_text(doc: &GenDoc, css: &str, root: usize) -> String {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(css, Origin::Author);
    let media = MediaContext::print();
    let inputs = dump::Inputs {
        dom: doc,
        tree: &tree,
        media: &media,
        custom_names: &[],
    };
    let mut out = String::new();
    first_line_into(&mut out, &inputs, StyleNodeId::new(root as u64));
    out
}

#[test]
fn the_first_line_entry_point_reports_every_outcome() {
    let mut doc = GenDoc::new(StyleQuirksMode::NoQuirks);
    let p = doc.append(0, GenNode::element("p"));
    let b = doc.append(p, GenNode::element("b"));
    doc.append(b, GenNode::text("bold"));
    let with = first_line_text(&doc, "p { display: block } p::first-line { color: red }", p);
    assert!(with.contains(&format!("first_line.root: {p}\n")), "{with}");
    assert!(
        with.contains(&format!("first_line[{b}]: ComputedValues")),
        "{with}"
    );
    let without = first_line_text(&doc, "p { display: block }", p);
    assert!(without.contains("first_line: None"), "{without}");
    let inline_root = first_line_text(&doc, "p::first-line { color: red }", p);
    assert!(inline_root.starts_with("ERR: "), "{inline_root}");
}

#[test]
fn a_seed_without_a_designated_block_runs_the_entry_point_for_its_first_element() {
    let seed = (0..100)
        .find(|&seed| generate::generate(seed).first_line_root.is_none())
        .expect("a seed in range has no designated block");
    let case = generate::generate(seed);
    let root = first_line_root(&case.doc);
    let text = render(&format!("gen:{seed}:first-line"), None);
    let unsupported = format!("unsupported first-line node {}", root.0);
    assert!(
        text.contains(&unsupported) || text.contains("first_line"),
        "seed {seed}: {}",
        text.lines().next().unwrap_or("")
    );
}

#[test]
fn every_designated_block_reaches_the_first_line_entry_point() {
    // The listed seeds hold important user-agent rules in layers, which beat
    // the designated block's guard while it was unlayered.
    for seed in (0..60).chain([794, 1133, 1569, 1588, 1705]) {
        let Some(root) = generate::generate(seed).first_line_root else {
            continue;
        };
        let text = render(&format!("gen:{seed}:first-line"), None);
        assert!(
            text.contains(&format!("first_line.root: {root}\n")),
            "seed {seed}: {}",
            text.lines().next().unwrap_or("")
        );
    }
}

#[test]
fn html_cases_run_the_first_line_entry_point_for_their_first_first_line_block() {
    let inline = TempFile::new(
        "first-line.html",
        "<!doctype html><style>p::first-line { color: red }</style><p>x <b>y</b></p>",
    );
    let text = render(&format!("html:{}", inline.path()), None);
    assert!(text.contains("first_line.root: "), "{text}");
    let nested = TempFile::new(
        "first-line-nested.html",
        "<!doctype html><style>div::first-line { color: red }</style><div><p>x</p></div>",
    );
    let text = render(&format!("html:{}", nested.path()), None);
    assert!(text.contains("first_line: ERR: "), "{text}");
    let plain = TempFile::new("plain.html", "<!doctype html><p>x</p>");
    let text = render(&format!("html:{}", plain.path()), None);
    assert!(!text.contains("first_line"), "{text}");
    // The outer block has a block child, so the inner one is tried next.
    let both = TempFile::new(
        "first-line-both.html",
        "<!doctype html><style>div::first-line, p::first-line { color: red }</style>\
         <div><p>x</p></div>",
    );
    let text = render(&format!("html:{}", both.path()), None);
    assert!(text.contains("first_line: ERR: "), "{text}");
    assert!(text.contains("first_line.root: "), "{text}");
}

#[test]
fn first_line_root_skips_non_elements() {
    let mut doc = GenDoc::new(StyleQuirksMode::NoQuirks);
    assert_eq!(first_line_root(&doc), doc.root_id());
    doc.append(0, GenNode::comment("c"));
    let html = doc.append(0, GenNode::element("html"));
    assert_eq!(first_line_root(&doc), StyleNodeId::new(html as u64));
    doc.append(html, GenNode::text("t"));
    let body = doc.append(html, GenNode::element("body"));
    assert_eq!(first_line_root(&doc), StyleNodeId::new(body as u64));
}
