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

fn command_arguments(print: bool, url: &str, output: &Path, hosts: &Path) -> Vec<String> {
    let mut arguments = Vec::new();
    if print {
        arguments.push("print-reftest".into());
    }
    arguments.extend([
        "--url".into(),
        url.into(),
        if print {
            "--output-directory"
        } else {
            "--output"
        }
        .into(),
        output.display().to_string(),
        if print {
            "--page-size"
        } else {
            "--window-size"
        }
        .into(),
        "32x32".into(),
        "--host-file".into(),
        hosts.display().to_string(),
    ]);
    arguments
}

#[test]
fn native_entrypoint_renders_screen_and_print_with_hosts_override() {
    use crate::test_http_server::TestServer;
    use std::collections::HashMap;

    let server = TestServer::start(HashMap::from([(
        "/test.html",
        (
            "text/html",
            b"<style>@page{size:32px 32px;margin:0}html,body{margin:0;background:lime}</style>"
                .to_vec(),
        ),
    )]));
    let mut url = server.url("test.html");
    url.set_host(Some("web-platform.test")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let hosts = directory.path().join("hosts");
    fs::write(&hosts, "127.0.0.1 web-platform.test\n").unwrap();
    for print in [false, true] {
        let output = directory
            .path()
            .join(if print { "pages" } else { "screen.png" });
        if print {
            fs::create_dir(&output).unwrap();
        }
        assert_eq!(
            entrypoint(command_arguments(print, url.as_str(), &output, &hosts)),
            ExitCode::SUCCESS
        );
        let png_path = if print {
            output.join("page-0001.png")
        } else {
            output
        };
        let decoder = png::Decoder::new(std::io::BufReader::new(fs::File::open(png_path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((frame.width, frame.height), (32, 32));
        assert!(
            pixels
                .chunks_exact(4)
                .any(|pixel| pixel == [0, 255, 0, 255])
        );
    }
    assert_eq!(server.finish(), ["/test.html", "/test.html"]);
}

#[test]
fn rejects_invalid_cli_arguments_for_both_commands() {
    for print in [false, true] {
        let valid = command_arguments(
            print,
            "http://web-platform.test/",
            Path::new("output"),
            Path::new("hosts"),
        );
        for (option, value, message) in [
            ("--url", "not a URL", "invalid --url"),
            ("--url", "https://web-platform.test/", "only http"),
            (
                if print {
                    "--page-size"
                } else {
                    "--window-size"
                },
                "32",
                "expected WIDTHxHEIGHT",
            ),
            (
                if print {
                    "--page-size"
                } else {
                    "--window-size"
                },
                "badx32",
                "invalid viewport width",
            ),
            (
                if print {
                    "--page-size"
                } else {
                    "--window-size"
                },
                "32xbad",
                "invalid viewport height",
            ),
            (
                if print {
                    "--page-size"
                } else {
                    "--window-size"
                },
                "0x32",
                "must be positive",
            ),
        ] {
            let mut args = valid.clone();
            let index = args.iter().position(|arg| arg == option).unwrap();
            args[index + 1] = value.into();
            assert!(run(args).unwrap_err().to_string().contains(message));
        }
        let mut args = valid.clone();
        args.extend(["--unknown".into(), "value".into()]);
        assert!(
            run(args)
                .unwrap_err()
                .to_string()
                .contains("unknown argument")
        );
        let mut args = valid.clone();
        args.extend(["--url".into(), "http://duplicate.test/".into()]);
        assert!(
            run(args)
                .unwrap_err()
                .to_string()
                .contains("duplicate argument")
        );
        let mut args = valid.clone();
        args.push("--url".into());
        assert!(run(args).unwrap_err().to_string().contains("missing value"));
        for option in [
            "--url",
            if print {
                "--output-directory"
            } else {
                "--output"
            },
            if print {
                "--page-size"
            } else {
                "--window-size"
            },
            "--host-file",
        ] {
            let mut args = valid.clone();
            let index = args.iter().position(|arg| arg == option).unwrap();
            args.drain(index..index + 2);
            assert!(
                run(args)
                    .unwrap_err()
                    .to_string()
                    .contains("missing required argument")
            );
        }
    }
    assert_eq!(entrypoint(Vec::new()), ExitCode::FAILURE);
}

#[test]
fn command_reports_missing_hosts_and_document_fetch_errors() {
    use crate::test_http_server::{TestResponse, TestServer};
    use std::collections::HashMap;

    let server = TestServer::start(HashMap::<&str, TestResponse>::new());
    let url = server.url("missing.html");
    let directory = tempfile::tempdir().unwrap();
    let hosts = directory.path().join("hosts");
    let output = directory.path().join("output");
    for print in [false, true] {
        if print {
            fs::create_dir(&output).unwrap();
        }
        let error = run(command_arguments(print, url.as_str(), &output, &hosts)).unwrap_err();
        assert!(error.to_string().contains("hosts"));
        fs::write(&hosts, "127.0.0.1 web-platform.test").unwrap();
        let error = run(command_arguments(print, url.as_str(), &output, &hosts)).unwrap_err();
        assert!(error.to_string().contains("document fetch failed"));
        fs::remove_file(&hosts).unwrap();
    }
    assert_eq!(server.finish(), ["/missing.html", "/missing.html"]);
}

#[test]
fn png_output_errors_remove_temporary_files() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("output.png");
    fs::create_dir(&output).unwrap();
    assert!(
        write_png_atomically(&output, &image(2, 2, 255))
            .unwrap_err()
            .to_string()
            .contains("rename")
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    fs::remove_dir(&output).unwrap();
    assert!(
        write_png_atomically(
            &output,
            &RenderedImage {
                width: 2,
                height: 2,
                rgba: vec![]
            }
        )
        .unwrap_err()
        .to_string()
        .contains("PNG data")
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    assert!(
        write_png_atomically(&output, &image(0, 0, 0))
            .unwrap_err()
            .to_string()
            .contains("PNG header")
    );
    assert!(
        write_png_atomically(
            &directory.path().join("missing/output.png"),
            &image(2, 2, 255)
        )
        .is_err()
    );
    assert!(
        write_png_atomically(Path::new("/"), &image(2, 2, 255))
            .unwrap_err()
            .to_string()
            .contains("no file name")
    );
    assert!(
        ensure_empty_output_directory(&directory.path().join("missing"))
            .unwrap_err()
            .to_string()
            .contains("does not exist")
    );
}
