use std::fs;
use std::path::PathBuf;

use super::*;
use crate::reftest::{RenderedDocument, RenderedImage};

fn image(width: u32, height: u32, value: u8) -> RenderedImage {
    RenderedImage {
        width,
        height,
        rgba: vec![value; width as usize * height as usize * 4],
    }
}

#[test]
fn parses_print_reftest_arguments() {
    let command = BrowserCommand::parse([
        "print-reftest",
        "--url",
        "http://web-platform.test:8000/test.html",
        "--output-directory",
        "/tmp/pages",
        "--page-size",
        "480x288",
        "--host-file",
        "/tmp/hosts",
    ])
    .expect("parse print command");

    let BrowserCommand::PrintReftest(args) = command else {
        panic!("expected print command");
    };
    assert_eq!(args.url.as_str(), "http://web-platform.test:8000/test.html");
    assert_eq!(args.output_directory, PathBuf::from("/tmp/pages"));
    assert_eq!((args.page_size.width, args.page_size.height), (480, 288));
    assert_eq!(args.host_file, PathBuf::from("/tmp/hosts"));
}

#[test]
fn rejects_nonempty_print_output_directory() {
    let directory = tempfile::tempdir().expect("temporary directory");
    fs::write(directory.path().join("unrelated.txt"), "occupied").expect("write marker");
    let document = RenderedDocument {
        pages: vec![image(2, 2, 0)],
    };

    let error = write_pages_atomically(directory.path(), &document)
        .expect_err("nonempty directory must fail");

    assert!(error.to_string().contains("must be empty"));
}

#[test]
fn writes_contiguous_numbered_pages_atomically() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let document = RenderedDocument {
        pages: vec![image(2, 2, 0), image(3, 1, 255)],
    };

    write_pages_atomically(directory.path(), &document).expect("write pages");

    let mut names: Vec<_> = fs::read_dir(directory.path())
        .expect("read output directory")
        .map(|entry| entry.expect("directory entry").file_name())
        .collect();
    names.sort();
    assert_eq!(names, ["page-0001.png", "page-0002.png"]);
    for name in names {
        let bytes = fs::read(directory.path().join(name)).expect("read PNG");
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}

#[test]
fn rejects_empty_print_document() {
    let directory = tempfile::tempdir().expect("temporary directory");

    let error = write_pages_atomically(directory.path(), &RenderedDocument::default())
        .expect_err("empty document must fail");

    assert!(error.to_string().contains("no pages"));
    assert_eq!(
        fs::read_dir(directory.path())
            .expect("read directory")
            .count(),
        0
    );
}

#[test]
fn keeps_legacy_screen_arguments() {
    let command = BrowserCommand::parse([
        "--url",
        "http://web-platform.test:8000/test.html",
        "--output",
        "/tmp/image.png",
        "--window-size",
        "800x600",
        "--host-file",
        "/tmp/hosts",
    ])
    .expect("parse legacy screen command");

    let BrowserCommand::Screen(args) = command else {
        panic!("expected screen command");
    };
    assert_eq!(args.output, PathBuf::from("/tmp/image.png"));
    assert_eq!((args.viewport.width, args.viewport.height), (800, 600));
}
