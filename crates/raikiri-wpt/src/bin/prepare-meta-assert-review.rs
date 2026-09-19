//! Prepare fixed-cost AI-agent review artifacts for WPT meta-assert tests.
//!
//! The binary deliberately does not call an AI API and never edits an
//! expectations file. It renders candidates once and writes a manifest that
//! an AI coding agent can inspect together with the copied source HTML and
//! screenshot.
//!
//! Usage:
//!
//! ```text
//! cargo run --locked -p raikiri-wpt --bin prepare-meta-assert-review -- \
//!     --wpt-root target/wpt \
//!     --baseline expectations/raikiri-baseline.txt \
//!     --output target/meta-assert-review
//! ```

use std::collections::HashSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use raikiri_wpt::reftest::parse_reftest_links;
use raikiri_wpt::reftest::render_raikiri;

const DEFAULT_WPT_ROOT: &str = "target/wpt";
const DEFAULT_BASELINE: &str = "expectations/meta-assert-baseline.txt";
const DEFAULT_EXCLUDE_BASELINE: &str = "expectations/raikiri-baseline.txt";
const DEFAULT_OUTPUT: &str = "target/meta-assert-review";
const DEFAULT_WIDTH: u32 = 800;
const DEFAULT_HEIGHT: u32 = 600;

#[derive(Debug, PartialEq, Eq)]
struct Args {
    wpt_root: PathBuf,
    baseline: PathBuf,
    exclude_baseline: PathBuf,
    output: PathBuf,
    limit: Option<usize>,
    width: u32,
    height: u32,
    include_parsing: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            wpt_root: PathBuf::from(DEFAULT_WPT_ROOT),
            baseline: PathBuf::from(DEFAULT_BASELINE),
            exclude_baseline: PathBuf::from(DEFAULT_EXCLUDE_BASELINE),
            output: PathBuf::from(DEFAULT_OUTPUT),
            limit: None,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            include_parsing: false,
        }
    }
}

#[derive(Debug)]
struct Candidate {
    test_id: String,
    source: String,
    assert_text: String,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let raw_args: Vec<String> = env::args().collect();
    if raw_args
        .iter()
        .skip(1)
        .any(|arg| arg == "--help" || arg == "-h")
    {
        println!("{}", usage());
        return Ok(());
    }
    let args = parse_args(&raw_args[1..])?;
    let mut baseline = read_baseline(&args.baseline)?;
    if args.exclude_baseline != args.baseline && args.exclude_baseline.exists() {
        baseline.extend(read_baseline(&args.exclude_baseline)?);
    }
    let wpt_sha = git_head(&args.wpt_root).unwrap_or_else(|| "unknown".to_owned());
    let candidates = discover_candidates(&args.wpt_root, &baseline, args.include_parsing)?;
    let candidates: Vec<_> = candidates
        .into_iter()
        .take(args.limit.unwrap_or(usize::MAX))
        .collect();

    fs::create_dir_all(args.output.join("html"))
        .map_err(|e| format!("could not create {}: {e}", args.output.display()))?;
    fs::create_dir_all(args.output.join("screenshots"))
        .map_err(|e| format!("could not create {}: {e}", args.output.display()))?;

    let manifest_path = args.output.join("manifest.jsonl");
    let template_path = args.output.join("reviews.template.jsonl");
    let mut manifest = String::new();
    let mut review_template = String::new();
    let mut rendered = 0usize;
    let mut errors = 0usize;

    for candidate in candidates {
        let html_rel = Path::new("html").join(&candidate.test_id);
        let screenshot_rel = Path::new("screenshots").join(format!("{}.png", candidate.test_id));
        let html_path = args.output.join(&html_rel);
        let screenshot_path = args.output.join(&screenshot_rel);
        if let Some(parent) = html_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
        if let Some(parent) = screenshot_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
        fs::write(&html_path, &candidate.source)
            .map_err(|e| format!("could not write {}: {e}", html_path.display()))?;

        let (status, error) =
            match render_png(&candidate.source, &screenshot_path, args.width, args.height) {
                Ok(()) => {
                    rendered += 1;
                    ("pending", None)
                }
                Err(error) => {
                    errors += 1;
                    ("render-error", Some(error))
                }
            };
        write_manifest_entry(
            &mut manifest,
            &candidate,
            &html_rel,
            &screenshot_rel,
            &wpt_sha,
            args.width,
            args.height,
            status,
            error.as_deref(),
        );
        if status == "pending" {
            let _ = writeln!(
                review_template,
                "{{\"test_id\":{},\"decision\":\"\",\"reason\":\"\"}}",
                json_string(&candidate.test_id)
            );
        }
    }

    fs::write(&manifest_path, manifest)
        .map_err(|e| format!("could not write {}: {e}", manifest_path.display()))?;
    fs::write(&template_path, review_template)
        .map_err(|e| format!("could not write {}: {e}", template_path.display()))?;
    println!(
        "Review artifacts written to {} ({} rendered, {} render errors; baseline unchanged)",
        args.output.display(),
        rendered,
        errors
    );
    Ok(())
}

fn usage() -> &'static str {
    "Usage: prepare-meta-assert-review [OPTIONS]\n\n  --wpt-root PATH       WPT checkout (default: target/wpt)\n  --baseline PATH       baseline file used for exclusion (default: expectations/raikiri-baseline.txt)\n  --output PATH         review artifact directory (default: target/meta-assert-review)\n  --limit N             process at most N candidates\n  --width N             viewport width (default: 800)\n  --height N            viewport height (default: 600)\n  --include-parsing     include tests below a parsing/ directory\n\nThe command writes manifest.jsonl, reviews.template.jsonl, html/, and screenshots/. It never edits the baseline."
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args::default();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        let (option, inline_value) = argument
            .split_once('=')
            .map_or((argument.as_str(), None), |(option, value)| {
                (option, Some(value))
            });
        let value = |index: &mut usize| -> Result<String, String> {
            if let Some(value) = inline_value {
                return Ok(value.to_owned());
            }
            *index += 1;
            args.get(*index)
                .cloned()
                .ok_or_else(|| format!("missing value for {option}\n\n{}", usage()))
        };
        match option {
            "--wpt-root" => parsed.wpt_root = PathBuf::from(value(&mut index)?),
            "--baseline" => parsed.baseline = PathBuf::from(value(&mut index)?),
            "--exclude-baseline" => parsed.exclude_baseline = PathBuf::from(value(&mut index)?),
            "--output" => parsed.output = PathBuf::from(value(&mut index)?),
            "--limit" => parsed.limit = Some(parse_usize(&value(&mut index)?, option)?),
            "--width" => parsed.width = parse_u32(&value(&mut index)?, option)?,
            "--height" => parsed.height = parse_u32(&value(&mut index)?, option)?,
            "--include-parsing" => {
                if inline_value.is_some() {
                    return Err(format!("{option} does not accept a value\n\n{}", usage()));
                }
                parsed.include_parsing = true;
            }
            other => return Err(format!("unknown argument {other}\n\n{}", usage())),
        }
        index += 1;
    }
    Ok(parsed)
}

fn parse_usize(raw: &str, option: &str) -> Result<usize, String> {
    raw.parse()
        .map_err(|_| format!("invalid value {raw:?} for {option}\n\n{}", usage()))
}

fn parse_u32(raw: &str, option: &str) -> Result<u32, String> {
    let value = raw
        .parse()
        .map_err(|_| format!("invalid value {raw:?} for {option}\n\n{}", usage()))?;
    if value == 0 {
        return Err(format!("{option} must be greater than zero\n\n{}", usage()));
    }
    Ok(value)
}

fn read_baseline(path: &Path) -> Result<HashSet<String>, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("could not read baseline {}: {e}", path.display()))?;
    Ok(content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect())
}

fn discover_candidates(
    wpt_root: &Path,
    baseline: &HashSet<String>,
    include_parsing: bool,
) -> Result<Vec<Candidate>, String> {
    let mut paths = Vec::new();
    collect_files(&wpt_root.join("css"), wpt_root, &mut paths).map_err(|e| {
        format!(
            "could not enumerate {}: {e}",
            wpt_root.join("css").display()
        )
    })?;
    paths.sort();

    let mut candidates = Vec::new();
    for path in paths {
        if !is_html_like(&path) {
            continue;
        }
        let test_id = path_to_string(&path);
        if !include_parsing && has_path_component(&test_id, "parsing") {
            continue;
        }
        if baseline.contains(&test_id) {
            continue;
        }
        let source = match fs::read_to_string(wpt_root.join(&path)) {
            Ok(source) => source,
            Err(_) => continue,
        };
        let Some(assert_text) = extract_meta_assert(&source) else {
            continue;
        };
        if !parse_reftest_links(&source).is_empty() {
            continue;
        }
        candidates.push(Candidate {
            test_id,
            source,
            assert_text,
        });
    }
    Ok(candidates)
}

fn collect_files(directory: &Path, root: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_files(&path, root, files)?;
        } else if entry.file_type()?.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("file is below WPT root")
                .to_path_buf();
            files.push(relative);
        }
    }
    Ok(())
}

fn is_html_like(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("html" | "htm" | "xhtml" | "xht")
    )
}

fn has_path_component(path: &str, component: &str) -> bool {
    path.split('/').any(|part| part == component)
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn extract_meta_assert(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut search = 0;
    while let Some(relative) = lower[search..].find("<meta") {
        let start = search + relative;
        let end = lower[start..].find('>')? + start;
        let tag = &html[start..=end];
        let tag_lower = &lower[start..=end];
        if attr(tag, tag_lower, "name")
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("assert"))
        {
            return attr(tag, tag_lower, "content").map(|value| value.trim().to_owned());
        }
        search = end + 1;
    }
    None
}

fn attr(tag: &str, tag_lower: &str, name: &str) -> Option<String> {
    let mut search = 0;
    while let Some(relative) = tag_lower[search..].find(name) {
        let start = search + relative;
        let before_ok = start == 0
            || tag_lower.as_bytes()[start - 1].is_ascii_whitespace()
            || tag_lower.as_bytes()[start - 1] == b'<'
            || tag_lower.as_bytes()[start - 1] == b'/';
        if !before_ok {
            search = start + name.len();
            continue;
        }
        let mut value_start = start + name.len();
        while tag_lower
            .as_bytes()
            .get(value_start)
            .is_some_and(u8::is_ascii_whitespace)
        {
            value_start += 1;
        }
        if tag_lower.as_bytes().get(value_start) != Some(&b'=') {
            search = value_start.saturating_add(1);
            continue;
        }
        value_start += 1;
        while tag_lower
            .as_bytes()
            .get(value_start)
            .is_some_and(u8::is_ascii_whitespace)
        {
            value_start += 1;
        }
        let quote = *tag_lower.as_bytes().get(value_start)?;
        if quote == b'\'' || quote == b'"' {
            let end = tag_lower[value_start + 1..].find(quote as char)? + value_start + 1;
            return Some(tag[value_start + 1..end].to_owned());
        }
        let mut end = value_start;
        while let Some(byte) = tag_lower.as_bytes().get(end) {
            if byte.is_ascii_whitespace() || *byte == b'>' {
                break;
            }
            end += 1;
        }
        return Some(tag[value_start..end].to_owned());
    }
    None
}

fn render_png(html: &str, output: &Path, width: u32, height: u32) -> Result<(), String> {
    let image = render_raikiri(html, width, height).map_err(|error| error.to_string())?;
    let file = fs::File::create(output).map_err(|e| format!("{}: {e}", output.display()))?;
    let writer = io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut png_writer = encoder
        .write_header()
        .map_err(|e| format!("PNG header: {e}"))?;
    png_writer
        .write_image_data(&image.rgba)
        .map_err(|e| format!("PNG data: {e}"))?;
    png_writer
        .finish()
        .map_err(|e| format!("PNG finish: {e}"))?;
    Ok(())
}

fn git_head(wpt_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(wpt_root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!sha.is_empty()).then_some(sha)
}

fn write_manifest_entry(
    output: &mut String,
    candidate: &Candidate,
    html_path: &Path,
    screenshot_path: &Path,
    wpt_sha: &str,
    width: u32,
    height: u32,
    status: &str,
    error: Option<&str>,
) {
    let html_path = path_to_string(html_path);
    let screenshot_path = path_to_string(screenshot_path);
    let _ = write!(
        output,
        "{{\"schema\":1,\"test_id\":{},\"html\":{},\"screenshot\":{},\"assert\":{},\"wpt_sha\":{},\"viewport\":{{\"width\":{},\"height\":{}}},\"renderer\":\"raikiri\",\"font_source\":\"wpt-root/fonts\",\"status\":{}",
        json_string(&candidate.test_id),
        json_string(&html_path),
        json_string(&screenshot_path),
        json_string(&candidate.assert_text),
        json_string(wpt_sha),
        width,
        height,
        json_string(status),
    );
    if let Some(error) = error {
        let _ = write!(output, ",\"error\":{}", json_string(error));
    }
    output.push_str("}\n");
}

fn json_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                let _ = write!(output, "\\u{:04x}", character as u32);
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_assert_with_attributes_in_either_order() {
        assert_eq!(
            extract_meta_assert(r#"<meta name='assert' content='one & two'>"#),
            Some("one & two".to_owned())
        );
        assert_eq!(
            extract_meta_assert(r#"<meta content="three" NAME="assert">"#),
            Some("three".to_owned())
        );
        assert_eq!(
            extract_meta_assert("<meta name='author' content='x'>"),
            None
        );
    }

    #[test]
    fn identifies_only_exact_parsing_path_components() {
        assert!(has_path_component("css/foo/parsing/a.html", "parsing"));
        assert!(!has_path_component(
            "css/foo/parsing-extra/a.html",
            "parsing"
        ));
    }

    #[test]
    fn json_string_escapes_manifest_values() {
        assert_eq!(json_string("a\"b\\c\n"), r#""a\"b\\c\n""#);
    }

    #[test]
    fn html_extension_filter_includes_wpt_variants() {
        assert!(is_html_like(Path::new("a.html")));
        assert!(is_html_like(Path::new("a.xht")));
        assert!(!is_html_like(Path::new("a.js")));
    }
}
