use super::*;

/// The font the tests lay documents out in.
pub(crate) const AHEM: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
);

/// A setup that lays documents out in [`AHEM`] and reads no files.
pub(crate) fn setup() -> Setup {
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", std::fs::read(AHEM).expect("the test font exists"))
        .build()
        .expect("the test font builds");
    Setup {
        fonts: Some(fonts),
        root: None,
    }
}

/// The canonical text of the HTML document `html`.
pub(crate) fn text_of(html: &str) -> String {
    lay_out(html.as_bytes(), None, &setup())
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

fn run_to_string(args: &[&str]) -> (ExitCode, String) {
    let mut out = Vec::new();
    let status = run(&strings(args), &mut out);
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

/// A directory in the system temporary directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("raikiri-layout-diff-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temporary directory");
        Self(path)
    }

    fn write(&self, name: &str, contents: &str) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, contents).expect("write temporary file");
        path.to_str().expect("utf-8 path").to_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn dump_prints_one_line_per_case() {
    let (status, out) = run_to_string(&["dump", "--font", AHEM, "--seeds", "0..4"]);
    assert_eq!(status, ExitCode::SUCCESS);
    assert_eq!(out.lines().count(), 4);
    for (index, line) in out.lines().enumerate() {
        let columns: Vec<&str> = line.split('\t').collect();
        assert_eq!(columns[0], format!("gen:{index}"));
        assert_eq!(columns[1].len(), 16, "{line}");
        match columns[2] {
            "ok" | "error" => assert_eq!(columns.len(), 3, "{line}"),
            "panic" => assert_eq!(columns.len(), 4, "{line}"),
            other => panic!("unexpected status {other}"),
        }
    }
}

#[test]
fn dump_without_cases_prints_nothing() {
    let (status, out) = run_to_string(&["dump"]);
    assert_eq!(status, ExitCode::SUCCESS);
    assert_eq!(out, "");
}

#[test]
fn dump_names_a_case_before_running_it() {
    let mut log = FlushLog::default();
    let args = strings(&["dump", "--font", AHEM, "--seeds", "0..2"]);
    assert_eq!(run(&args, &mut log), ExitCode::SUCCESS);
    assert_eq!(log.flushed[0], "gen:0\t");
    assert!(log.flushed[1].starts_with("gen:0\t"));
    assert!(log.flushed[1].ends_with('\n'));
    assert_eq!(log.flushed[2], format!("{}gen:1\t", log.flushed[1]));
}

#[test]
fn dump_is_repeatable_and_does_not_depend_on_the_order_of_cases() {
    let (_, forward) = run_to_string(&["dump", "--font", AHEM, "--seeds", "0..16"]);
    let (_, again) = run_to_string(&["dump", "--font", AHEM, "--seeds", "0..16"]);
    assert_eq!(forward, again);
    // The same cases one at a time, last first: nothing a case leaves behind
    // in the process may reach the next.
    let mut backward = Vec::new();
    for seed in (0..16u64).rev() {
        let range = format!("{seed}..{}", seed + 1);
        let (_, line) = run_to_string(&["dump", "--font", AHEM, "--seeds", &range]);
        backward.push(line);
    }
    backward.reverse();
    assert_eq!(forward, backward.concat());
}

#[test]
fn dump_lays_out_html_files_listed_for_it() {
    let dir = TempDir::new("html-list");
    let page = dir.write("page.html", "<p style='width:40px'>hello</p>");
    let list = dir.write(
        "list.txt",
        &format!("{page}\n\n{}/missing.html\n", dir.0.display()),
    );
    let root = dir.0.to_str().unwrap();
    let (status, out) = run_to_string(&[
        "dump",
        "--font",
        AHEM,
        "--html-list",
        &list,
        "--html-root",
        root,
    ]);
    assert_eq!(status, ExitCode::SUCCESS);
    let lines: Vec<Vec<&str>> = out.lines().map(|line| line.split('\t').collect()).collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0][0], format!("html:{page}"));
    assert_eq!(lines[0][2], "ok");
    assert_eq!(lines[1][2], "error");
}

#[test]
fn command_lines_that_cannot_run_exit_with_status_2() {
    let dir = TempDir::new("dump-usage");
    let missing = format!("{}/none.txt", dir.0.display());
    let not_a_font = dir.write("font.ttf", "not a font");
    let tabbed = dir.write("tabbed.txt", "a page\twith a tab.html\n");
    for args in [
        &["dump", "extra"][..],
        &["dump", "--seeds"][..],
        &["dump", "--seeds", "7"][..],
        &["dump", "--seeds", "a..b"][..],
        &["dump", "--nothing", "x"][..],
        &["dump", "--html-list", &missing][..],
        &["dump", "--html-list", &tabbed][..],
        &["dump", "--font", &missing][..],
        &["dump", "--font", &not_a_font][..],
        &["show"][..],
        &["show", "gen:0", "gen:1"][..],
        &["show", "--seeds", "0..1", "gen:0"][..],
        &["show", "--font", &missing, "gen:0"][..],
        &["describe"][..],
        &["describe", "x"][..],
        &["describe", "1", "2"][..],
        &["compiler", "extra"][..],
        &["other"][..],
        &[][..],
    ] {
        let (status, out) = run_to_string(args);
        assert_eq!(status, ExitCode::from(2), "{args:?}");
        assert_eq!(out, "", "{args:?}");
    }
}

#[test]
fn a_failing_writer_fails_every_command() {
    for args in [
        &["dump", "--seeds", "0..1"][..],
        &["show", "gen:0"][..],
        &["describe", "0"][..],
        &["compiler"][..],
    ] {
        assert_eq!(
            run(&strings(args), &mut FailingWriter),
            ExitCode::FAILURE,
            "{args:?}"
        );
    }
}

#[test]
fn show_prints_the_canonical_text_that_dump_hashes() {
    let (status, text) = run_to_string(&["show", "--font", AHEM, "gen:3"]);
    assert_eq!(status, ExitCode::SUCCESS);
    assert!(text.starts_with("pages: "), "{text}");
    let (_, line) = run_to_string(&["dump", "--font", AHEM, "--seeds", "3..4"]);
    assert_eq!(
        line,
        format!("gen:3\t{}\n", case_columns(&text)),
        "show and dump agree on the case"
    );
}

#[test]
fn show_reads_html_cases_under_the_root() {
    let dir = TempDir::new("show-html");
    std::fs::create_dir_all(dir.0.join("css")).unwrap();
    dir.write("css/wide.css", "p { width: 123px; margin: 0 }");
    let page = dir.write(
        "page.html",
        "<link rel=stylesheet href='/css/wide.css'><p>hello</p>",
    );
    let root = dir.0.to_str().unwrap();
    let case = format!("html:{page}");
    let (status, text) = run_to_string(&["show", "--font", AHEM, "--html-root", root, &case]);
    assert_eq!(status, ExitCode::SUCCESS);
    assert!(
        text.contains("size=[123.0 "),
        "the linked sheet applies: {text}"
    );
}

#[test]
fn describe_prints_the_generated_document() {
    let (status, out) = run_to_string(&["describe", "9"]);
    assert_eq!(status, ExitCode::SUCCESS);
    assert_eq!(out, generate::generate(9));
}

#[test]
fn compiler_prints_the_compiler_version() {
    let (status, out) = run_to_string(&["compiler"]);
    assert_eq!(status, ExitCode::SUCCESS);
    assert!(out.starts_with("rustc "), "{out}");
}

#[test]
fn case_columns_name_the_status() {
    let text = "PANIC: a\tb\nsecond line\n";
    assert_eq!(
        case_columns(text),
        format!("{:016x}\tpanic\ta b", dump::fnv1a(text))
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
    let text = "pages: 1\n";
    assert_eq!(
        case_columns(text),
        format!("{:016x}\tok", dump::fnv1a(text))
    );
}

#[test]
fn ranges_have_a_start_and_an_end() {
    assert_eq!(parse_range("2..9"), Some(2..9));
    assert_eq!(parse_range("2"), None);
    assert_eq!(parse_range("..9"), None);
    assert_eq!(parse_range("2.."), None);
}

#[test]
fn arguments_keep_flags_and_other_words_apart() {
    let args = strings(&["--a", "1", "word", "--b", "2", "--a", "3"]);
    let parsed = Arguments::parse(&args, &["--a", "--b"]).expect("parses");
    assert_eq!(parsed.positional, ["word"]);
    assert_eq!(parsed.values("--a").collect::<Vec<_>>(), ["1", "3"]);
    assert_eq!(parsed.last("--a"), Some("3"));
    assert_eq!(parsed.last("--b"), Some("2"));
    assert_eq!(parsed.last("--c"), None);
    assert!(Arguments::parse(&strings(&["--a"]), &["--a"]).is_none());
    assert!(Arguments::parse(&strings(&["--c", "1"]), &["--a"]).is_none());
}

#[test]
fn a_setup_names_the_file_it_cannot_use() {
    let dir = TempDir::new("setup");
    let not_a_font = dir.write("font.ttf", "not a font");
    let missing = format!("{}/none.ttf", dir.0.display());
    for (path, expected) in [(&missing, "cannot read"), (&not_a_font, "as a font")] {
        let args = strings(&["--font", path]);
        let arguments = Arguments::parse(&args, &["--font"]).unwrap();
        let error = Setup::from_arguments(&arguments)
            .err()
            .expect("the font is refused");
        assert!(
            error.contains(expected) && error.contains(path.as_str()),
            "{error}"
        );
    }
    let args = strings(&["--font", AHEM, "--html-root", "/some/root"]);
    let arguments = Arguments::parse(&args, &["--font", "--html-root"]).unwrap();
    let setup = Setup::from_arguments(&arguments).expect("a usable setup");
    assert!(setup.fonts.is_some());
    assert_eq!(setup.root, Some(PathBuf::from("/some/root")));
    let args = strings(&[]);
    let arguments = Arguments::parse(&args, &[]).unwrap();
    let setup = Setup::from_arguments(&arguments).expect("the empty setup");
    assert!(setup.fonts.is_none() && setup.root.is_none());
}

#[test]
fn cases_that_cannot_run_say_so() {
    let setup = setup();
    assert_eq!(render("gen:x", &setup), "BAD CASE: gen:x\n");
    assert_eq!(render("other", &setup), "BAD CASE: other\n");
    assert!(render("html:/nonexistent/page.html", &setup).starts_with("IO-ERROR: "));
    // Text that is not UTF-8 does not parse.
    assert!(lay_out(&[0xff, 0xfe, 0xfd], None, &setup).starts_with("PARSE-ERROR: "));
}

#[test]
fn a_panic_becomes_the_text_of_its_case() {
    assert_eq!(
        guarded(|| panic!("static message")),
        "PANIC: static message\n"
    );
    let detail = 7;
    assert_eq!(
        guarded(|| panic!("formatted {detail}")),
        "PANIC: formatted 7\n"
    );
    assert_eq!(
        guarded(|| std::panic::panic_any(7_u8)),
        "PANIC: <non-string panic payload>\n"
    );
    assert_eq!(guarded(|| "fine".to_owned()), "fine");
}

#[test]
fn only_a_running_case_counts_as_being_in_a_case() {
    assert!(!in_case());
    assert_eq!(guarded(|| in_case().to_string()), "true");
    assert!(!in_case());
    let _ = guarded(|| panic!("after this the flag is cleared"));
    assert!(!in_case());
}

#[test]
fn the_panic_hook_is_silent_only_inside_a_case() {
    install_panic_hook();
    // Both kinds of panic are caught, so nothing escapes the test; which of
    // them the hook lets through to the default hook shows on standard error
    // only.
    assert_eq!(
        guarded(|| panic!("inside a case")),
        "PANIC: inside a case\n"
    );
    let outside = std::thread::spawn(|| panic!("outside a case")).join();
    assert!(outside.is_err());
}

#[test]
fn a_layout_that_did_not_complete_says_so() {
    assert_eq!(
        layout_text(Ok(LayoutStatus::Aborted)),
        "ERR: the layout did not complete\n"
    );
    let text = layout_text(Err(RenderError::Configuration("no print".to_owned())));
    assert_eq!(text, "ERR: Configuration(\"no print\")\n");
}

#[test]
fn documents_are_laid_out_on_an_800_by_600_page_unless_they_say_otherwise() {
    let text = text_of("<p>hello</p>");
    assert!(text.contains("width: 800.0, height: 600.0"), "{text}");
    let text = text_of("<style>@page { size: 300px 200px }</style><p>hello</p>");
    assert!(text.contains("width: 300.0, height: 200.0"), "{text}");
}

#[test]
// `cfg!` is a constant for the lint, but the cfg comes from build.rs, so the
// assert is what fails the run when the probe stops finding the reader.
#[allow(clippy::assertions_on_constants)]
fn the_tree_the_tool_is_tested_in_has_a_style_reader() {
    // `build.rs` finds the reader by reading the sources of `raikiri-dom`. A
    // tree that loses it writes no style getters, and so would a tree that the
    // search no longer finds it in: the comparison would pass with nothing
    // compared. This tree has the reader, so the cfg must be set.
    assert!(
        cfg!(style_view_reader),
        "build.rs no longer finds the style reader of raikiri-dom"
    );
}
