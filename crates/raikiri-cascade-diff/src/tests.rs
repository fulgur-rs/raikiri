use super::*;
use crate::doc::{GenDoc, GenNode};
use raikiri_style::StyleQuirksMode;

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
            "ok" => assert_eq!(columns.len(), 3, "{line}"),
            "panic" => assert_eq!(columns.len(), 4, "{line}"),
            other => panic!("unexpected status {other}"),
        }
    }
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
    assert!(render(&format!("html:{}", html.path())).starts_with("root_element_index:"));
    assert!(render("html:/nonexistent/raikiri-cascade-diff.html").starts_with("IO-ERROR:"));
}

#[test]
fn invalid_arguments_are_usage_errors() {
    for args in [
        &[][..],
        &["show"][..],
        &["describe", "x"][..],
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
    assert_eq!(out, render("gen:3:print"));
    let (status, out) = run_to_string(&["describe", "3"]);
    assert_eq!(status, ExitCode::SUCCESS);
    assert!(out.contains("/* Author */"));
}

#[test]
fn write_failures_are_reported() {
    let args = vec!["dump".to_owned(), "--seeds".to_owned(), "0..1".to_owned()];
    assert_eq!(run(&args, &mut FailingWriter), ExitCode::FAILURE);
    let args = vec!["show".to_owned(), "gen:0:print".to_owned()];
    assert_eq!(run(&args, &mut FailingWriter), ExitCode::FAILURE);
}

#[test]
fn malformed_case_ids_are_reported() {
    for case in ["gen:3", "gen:x:print", "gen:3:bogus", "other:3"] {
        assert_eq!(render(case), format!("BAD CASE: {case}\n"));
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

#[test]
fn first_line_cases_cover_every_outcome() {
    let (mut errors, mut without, mut with) = (0, 0, 0);
    for seed in 0..400 {
        let text = render(&format!("gen:{seed}:first-line"));
        if text.starts_with("ERR:") {
            errors += 1;
        } else if text.contains("first_line: None") {
            without += 1;
        } else if text.contains("first_line.root:") {
            with += 1;
        }
    }
    assert!(
        errors > 0 && without > 0 && with > 0,
        "{errors} {without} {with}"
    );
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
