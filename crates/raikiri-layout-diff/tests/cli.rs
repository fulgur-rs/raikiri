//! The tool as a process: what a script that runs it sees on standard output
//! and in the exit status.

use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_raikiri-layout-diff");

/// The font the cases are laid out in.
const AHEM: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
);

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("run raikiri-layout-diff")
}

#[test]
fn describe_prints_the_document_of_a_seed() {
    let output = run(&["describe", "3"]);
    assert!(output.status.success());
    let html = String::from_utf8(output.stdout).expect("utf-8 output");
    assert!(html.starts_with("<!doctype html>"), "{html}");
}

#[test]
fn dump_prints_a_line_per_case_and_is_the_same_every_time() {
    let args = ["dump", "--font", AHEM, "--seeds", "0..3"];
    let first = run(&args);
    assert!(first.status.success());
    let text = String::from_utf8(first.stdout.clone()).expect("utf-8 output");
    let cases: Vec<&str> = text
        .lines()
        .map(|line| line.split('\t').next().unwrap())
        .collect();
    assert_eq!(cases, ["gen:0", "gen:1", "gen:2"]);
    assert!(
        text.lines().all(|line| line.ends_with("\tok")),
        "every generated case lays out:\n{text}"
    );
    assert_eq!(first.stdout, run(&args).stdout);
}

#[test]
fn a_command_line_it_cannot_run_exits_with_status_2() {
    let output = run(&[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let usage = String::from_utf8(output.stderr).expect("utf-8 output");
    assert!(
        usage.starts_with("usage: raikiri-layout-diff dump"),
        "{usage}"
    );
}

/// Output that cannot be written is a failure, not a shorter report.
#[cfg(target_os = "linux")]
#[test]
fn output_that_cannot_be_written_fails_the_run() {
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");
    let status = Command::new(BIN)
        .args(["describe", "3"])
        .stdout(full)
        .status()
        .expect("run raikiri-layout-diff");
    assert_eq!(status.code(), Some(1));
}
