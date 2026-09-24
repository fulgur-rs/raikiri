//! Executes the testharness-only tests in `css/css-text/i18n` against Raikiri.
//!
//! Reftest files in the same directory remain on the visual runner. This
//! module covers testharness scripts that query DOM geometry.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use raikiri_js::TestOutcome;
use raikiri_js::dom::{DomBackend, DomNodeId, DomRect, DomSnapshot, ElementGeometry};
use raikiri_js::testharness::run_testharness_script;

use crate::reftest::{
    DEFAULT_REFTTEST_HEIGHT, DEFAULT_REFTTEST_WIDTH, layout_raikiri_wpt_document,
};

const TEST_DIR: &str = "css/css-text/i18n";

/// Per-file result from a CSS Text i18n testharness page.
#[derive(Debug)]
pub struct TestHarnessFileResult {
    /// The WPT-relative test ID.
    pub test_id: String,
    /// Results for every `test()` call. Empty only when `error` is set.
    pub outcomes: Vec<TestOutcome>,
    /// A parse, layout, or JavaScript harness error, separate from assertion failures.
    pub error: Option<String>,
}

impl TestHarnessFileResult {
    /// Number of passing assertions.
    pub fn passed(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|outcome| outcome.passed)
            .count()
    }

    /// Number of assertions that ran.
    pub fn total(&self) -> usize {
        self.outcomes.len()
    }

    /// Whether all assertions ran and passed.
    pub fn all_passed(&self) -> bool {
        self.error.is_none() && self.total() > 0 && self.passed() == self.total()
    }
}

/// Why the CSS Text i18n test suite could not be discovered or executed.
#[derive(Debug)]
pub enum TestHarnessRunError {
    /// The selected WPT directory is missing or cannot be read.
    Io(String),
    /// The pinned tree had no testharness pages in the selected directory.
    NoTests,
}

impl std::fmt::Display for TestHarnessRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) => write!(f, "I/O error: {message}"),
            Self::NoTests => write!(f, "no testharness pages found under {TEST_DIR}"),
        }
    }
}

impl std::error::Error for TestHarnessRunError {}

fn body_subtree(snapshot: &DomSnapshot) -> BTreeSet<DomNodeId> {
    let mut children_by_parent: BTreeMap<DomNodeId, Vec<DomNodeId>> = BTreeMap::new();
    for (node, snapshot_node) in &snapshot.nodes {
        if let Some(parent) = snapshot_node.parent {
            children_by_parent.entry(parent).or_default().push(*node);
        }
    }

    let mut subtree = BTreeSet::new();
    let mut pending = snapshot.body.into_iter().collect::<Vec<_>>();
    while let Some(node) = pending.pop() {
        if subtree.insert(node)
            && let Some(children) = children_by_parent.get(&node)
        {
            pending.extend(children.iter().copied());
        }
    }
    subtree
}

/// Initial adapter from renderer snapshots to the reusable `raikiri-js` DOM
/// backend. A live Raikiri DOM document backend can replace this without
/// changing the JavaScript facade or WPT script source.
struct LayoutSnapshotBackend<F> {
    snapshot: DomSnapshot,
    source_to_handle: BTreeMap<DomNodeId, DomNodeId>,
    handle_to_source: BTreeMap<DomNodeId, DomNodeId>,
    next_handle: DomNodeId,
    body_html: String,
    styles: BTreeMap<(DomNodeId, String), String>,
    layout_after_body_inner_html: F,
}

impl<F> LayoutSnapshotBackend<F>
where
    F: FnMut(&str) -> Result<DomSnapshot, String>,
{
    fn new(snapshot: DomSnapshot, layout_after_body_inner_html: F) -> Self {
        let mut backend = Self {
            snapshot: DomSnapshot::default(),
            source_to_handle: BTreeMap::new(),
            handle_to_source: BTreeMap::new(),
            next_handle: 1,
            body_html: String::new(),
            styles: BTreeMap::new(),
            layout_after_body_inner_html,
        };
        backend.replace_snapshot(snapshot, None);
        backend
    }

    fn allocate_handle(&mut self) -> DomNodeId {
        let handle = self.next_handle;
        self.next_handle = self
            .next_handle
            .checked_add(1)
            .expect("DOM snapshot exhausted u64 node handles");
        handle
    }

    fn replace_snapshot(
        &mut self,
        snapshot: DomSnapshot,
        preserved_body_handle: Option<DomNodeId>,
    ) {
        // Source NodeIds belong to one parsed Document and may be reused after
        // body.innerHTML is reparsed. Reissue handles in the replaced body
        // subtree, preserve the body object, and retain unaffected nodes outside
        // the body when their source NodeIds remain stable.
        let old_body_subtree = body_subtree(&self.snapshot);
        let new_body_subtree = body_subtree(&snapshot);
        let old_source_to_handle = std::mem::take(&mut self.source_to_handle);
        let mut source_to_handle = BTreeMap::new();
        let mut handle_to_source = BTreeMap::new();
        for source in snapshot.nodes.keys().copied() {
            let handle = if Some(source) == snapshot.body {
                preserved_body_handle.unwrap_or_else(|| self.allocate_handle())
            } else if old_body_subtree.contains(&source) || new_body_subtree.contains(&source) {
                self.allocate_handle()
            } else if let Some(handle) = old_source_to_handle.get(&source).copied() {
                handle
            } else {
                self.allocate_handle()
            };
            source_to_handle.insert(source, handle);
            handle_to_source.insert(handle, source);
        }
        self.styles
            .retain(|(handle, _), _| handle_to_source.contains_key(handle));
        self.snapshot = snapshot;
        self.source_to_handle = source_to_handle;
        self.handle_to_source = handle_to_source;
    }

    fn source_for_handle(&self, handle: DomNodeId) -> Result<DomNodeId, String> {
        self.handle_to_source
            .get(&handle)
            .copied()
            .ok_or_else(|| format!("stale DOM node handle {handle}"))
    }

    fn handle_for_source(&self, source: DomNodeId) -> Result<DomNodeId, String> {
        self.source_to_handle
            .get(&source)
            .copied()
            .ok_or_else(|| format!("no JS handle for DOM source node {source}"))
    }

    fn geometry(&self, handle: DomNodeId) -> Result<ElementGeometry, String> {
        let source = self.source_for_handle(handle)?;
        self.snapshot
            .nodes
            .get(&source)
            .map(|node| node.geometry)
            .ok_or_else(|| format!("no DOM element for node handle {handle}"))
    }
}

impl<F> DomBackend for LayoutSnapshotBackend<F>
where
    F: FnMut(&str) -> Result<DomSnapshot, String> + 'static,
{
    fn get_element_by_id(&mut self, id: &str) -> Result<Option<DomNodeId>, String> {
        self.snapshot
            .elements_by_id
            .get(id)
            .copied()
            .map(|source| self.handle_for_source(source))
            .transpose()
    }

    fn query_selector(&mut self, selector: &str) -> Result<Option<DomNodeId>, String> {
        let selector = selector.trim();
        if selector.eq_ignore_ascii_case("body") {
            return self
                .snapshot
                .body
                .map(|source| self.handle_for_source(source))
                .transpose();
        }
        if let Some(id) = selector.strip_prefix('#') {
            return self.get_element_by_id(id);
        }
        Ok(None)
    }

    fn parent_node(&mut self, node: DomNodeId) -> Result<Option<DomNodeId>, String> {
        let source = self.source_for_handle(node)?;
        let parent = self
            .snapshot
            .nodes
            .get(&source)
            .ok_or_else(|| format!("no DOM element for node handle {node}"))?
            .parent;
        parent
            .map(|source| self.handle_for_source(source))
            .transpose()
    }

    fn offset_height(&mut self, node: DomNodeId) -> Result<f64, String> {
        Ok(self.geometry(node)?.offset_height)
    }

    fn bounding_client_rect(&mut self, node: DomNodeId) -> Result<DomRect, String> {
        Ok(self.geometry(node)?.bounding_client_rect)
    }

    fn inner_html(&mut self, node: DomNodeId) -> Result<String, String> {
        if self.snapshot.body != Some(self.source_for_handle(node)?) {
            return Err("innerHTML is only available on body in the snapshot backend".into());
        }
        Ok(self.body_html.clone())
    }

    fn set_inner_html(&mut self, node: DomNodeId, value: &str) -> Result<(), String> {
        if self.snapshot.body != Some(self.source_for_handle(node)?) {
            return Err(
                "innerHTML mutation is only available on body in the snapshot backend".into(),
            );
        }
        let snapshot = (self.layout_after_body_inner_html)(value)?;
        self.replace_snapshot(snapshot, Some(node));
        self.body_html = value.to_owned();
        Ok(())
    }

    fn style_property(&mut self, node: DomNodeId, property: &str) -> Result<String, String> {
        self.source_for_handle(node)?;
        Ok(self
            .styles
            .get(&(node, property.to_owned()))
            .cloned()
            .unwrap_or_default())
    }

    fn set_style_property(
        &mut self,
        node: DomNodeId,
        property: &str,
        value: &str,
    ) -> Result<(), String> {
        self.source_for_handle(node)?;
        self.styles
            .insert((node, property.to_owned()), value.to_owned());
        Ok(())
    }
}

/// Run every testharness-only HTML page under `css/css-text/i18n`.
///
/// Each script sees real Raikiri layout metrics. If a script replaces the
/// body's `innerHTML`, the new body is inserted into the original page's head
/// and laid out again before its `document.fonts.ready` callbacks run.
pub fn run_css_text_i18n(
    wpt_root: &Path,
) -> Result<Vec<TestHarnessFileResult>, TestHarnessRunError> {
    let test_root = wpt_root.join(TEST_DIR);
    if !test_root.is_dir() {
        return Err(TestHarnessRunError::Io(format!(
            "{} is not a directory",
            test_root.display()
        )));
    }

    let files = discover_testharness_files(&test_root)?;
    if files.is_empty() {
        return Err(TestHarnessRunError::NoTests);
    }

    let results = files
        .iter()
        .map(|path| run_testharness_file(path, wpt_root))
        .collect();
    Ok(results)
}

fn run_testharness_file(path: &Path, wpt_root: &Path) -> TestHarnessFileResult {
    let test_id = path
        .strip_prefix(wpt_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let html = match fs::read_to_string(path) {
        Ok(html) => html,
        Err(error) => {
            return TestHarnessFileResult {
                test_id,
                outcomes: Vec::new(),
                error: Some(format!("read test HTML: {error}")),
            };
        }
    };
    let script = inline_test_scripts(&html);
    if !script.contains("test(") {
        return TestHarnessFileResult {
            test_id,
            outcomes: Vec::new(),
            error: Some("testharness.js is referenced but no inline test() call was found".into()),
        };
    }

    let initial_snapshot = match layout_raikiri_wpt_document(
        &html,
        DEFAULT_REFTTEST_WIDTH,
        DEFAULT_REFTTEST_HEIGHT,
        wpt_root,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return TestHarnessFileResult {
                test_id,
                outcomes: Vec::new(),
                error: Some(format!("initial Raikiri layout: {error}")),
            };
        }
    };

    let original_html = html.clone();
    let wpt_root = wpt_root.to_path_buf();
    let backend = LayoutSnapshotBackend::new(initial_snapshot, move |body_html| {
        let updated_html = replace_body_inner_html(&original_html, body_html)?;
        layout_raikiri_wpt_document(
            &updated_html,
            DEFAULT_REFTTEST_WIDTH,
            DEFAULT_REFTTEST_HEIGHT,
            &wpt_root,
        )
        .map_err(|error| error.to_string())
    });
    let result = run_testharness_script(&script, backend);
    match result {
        Ok(outcomes) => TestHarnessFileResult {
            test_id,
            outcomes,
            error: None,
        },
        Err(error) => TestHarnessFileResult {
            test_id,
            outcomes: Vec::new(),
            error: Some(error.to_string()),
        },
    }
}

fn inline_test_scripts(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut output = String::new();
    let mut search = 0usize;

    while search < html.len() {
        let comment = lower[search..].find("<!--").map(|offset| search + offset);
        let script = lower[search..]
            .find("<script")
            .map(|offset| search + offset);
        let Some(script_start) = script else {
            if let Some(comment_start) = comment {
                let Some(comment_end) = lower[comment_start + 4..].find("-->") else {
                    break;
                };
                search = comment_start + 4 + comment_end + 3;
                continue;
            }
            break;
        };

        if comment.is_some_and(|comment_start| comment_start < script_start) {
            let comment_start = comment.expect("checked above");
            let Some(comment_end) = lower[comment_start + 4..].find("-->") else {
                break;
            };
            search = comment_start + 4 + comment_end + 3;
            continue;
        }

        let Some(tag_end_offset) = lower[script_start..].find('>') else {
            break;
        };
        let tag_end = script_start + tag_end_offset;
        let tag = &lower[script_start..=tag_end];
        let has_src = tag.split_whitespace().any(|attribute| {
            attribute
                .strip_prefix("src")
                .is_some_and(|suffix| suffix.trim_start().starts_with('='))
        });
        let body_start = tag_end + 1;
        let Some(close_offset) = lower[body_start..].find("</script>") else {
            break;
        };
        let body_end = body_start + close_offset;
        if !has_src {
            output.push_str(&html[body_start..body_end]);
            output.push('\n');
        }
        search = body_end + "</script>".len();
    }

    output
}

fn discover_testharness_files(test_root: &Path) -> Result<Vec<PathBuf>, TestHarnessRunError> {
    let mut stack = vec![test_root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = stack.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| {
            TestHarnessRunError::Io(format!("{}: {error}", directory.display()))
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| TestHarnessRunError::Io(error.to_string()))?;
            let path = entry.path();
            if path.is_dir() {
                if path
                    .file_name()
                    .is_some_and(|name| name == "reference" || name == "support")
                {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
                continue;
            };
            if !matches!(
                extension.to_ascii_lowercase().as_str(),
                "html" | "htm" | "xhtml"
            ) {
                continue;
            }
            let source = fs::read_to_string(&path)
                .map_err(|error| TestHarnessRunError::Io(format!("{}: {error}", path.display())))?;
            if source.to_ascii_lowercase().contains("testharness.js") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn replace_body_inner_html(html: &str, body_html: &str) -> Result<String, String> {
    let lower = html.to_ascii_lowercase();
    let body_start = lower
        .find("<body")
        .ok_or_else(|| "test page has no <body> start tag".to_owned())?;
    let open_end = lower[body_start..]
        .find('>')
        .map(|index| body_start + index + 1)
        .ok_or_else(|| "test page has an unterminated <body> start tag".to_owned())?;
    let close_start = lower[open_end..]
        .find("</body>")
        .map(|index| open_end + index)
        .ok_or_else(|| "test page has no </body> end tag".to_owned())?;
    let mut updated = String::with_capacity(html.len() + body_html.len());
    updated.push_str(&html[..open_end]);
    updated.push_str(body_html);
    updated.push_str(&html[close_start..]);
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_testharness_files_recursively_but_not_reference_documents() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("css/css-text/i18n");
        fs::create_dir_all(root.join("zh/reference")).unwrap();
        fs::write(
            root.join("test.html"),
            "<script src='/resources/testharness.js'></script>",
        )
        .unwrap();
        fs::write(
            root.join("zh/locale.html"),
            "<script src='/resources/testharness.js'></script>",
        )
        .unwrap();
        fs::write(
            root.join("zh/reference/ref.html"),
            "<script src='/resources/testharness.js'></script>",
        )
        .unwrap();
        fs::write(
            root.join("visual.html"),
            "<link rel='match' href='ref.html'>",
        )
        .unwrap();

        let discovered = discover_testharness_files(&root).unwrap();
        let ids = discovered
            .iter()
            .map(|path| {
                path.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, ["test.html", "zh/locale.html"]);
    }

    #[test]
    fn inline_script_extraction_ignores_html_comment_examples() {
        let html = "<!-- <script>not_a_test();</script> -->\n<script>test(function() {}, 'real');</script>";
        let scripts = inline_test_scripts(html);
        assert!(scripts.contains("test(function()"));
        assert!(!scripts.contains("not_a_test"));
    }

    #[test]
    fn body_inner_html_replaces_only_the_original_body_contents() {
        let html = "<html><head><style>body { color: red }</style></head><body><script>x()</script></body></html>";
        let updated = replace_body_inner_html(html, "<div id='x'>text</div>").unwrap();
        assert!(updated.contains("<style>body { color: red }</style>"));
        assert!(updated.contains("<body><div id='x'>text</div></body>"));
        assert!(!updated.contains("x()"));
    }

    #[test]
    fn missing_body_start_or_end_is_an_error() {
        assert!(replace_body_inner_html("<html></html>", "x").is_err());
        assert!(replace_body_inner_html("<body>x", "y").is_err());
    }

    fn write_test_page(wpt_root: &Path, name: &str, html: &str) {
        let test_dir = wpt_root.join(TEST_DIR);
        fs::create_dir_all(&test_dir).unwrap();
        fs::write(test_dir.join(name), html).unwrap();
    }

    #[test]
    fn body_relayout_never_reuses_handles_for_reparsed_source_node_ids() {
        use raikiri_js::dom::SnapshotNode;

        fn snapshot(child_id: &str) -> DomSnapshot {
            DomSnapshot {
                nodes: BTreeMap::from([
                    (
                        9,
                        SnapshotNode {
                            parent: None,
                            geometry: ElementGeometry::default(),
                        },
                    ),
                    (
                        10,
                        SnapshotNode {
                            parent: None,
                            geometry: ElementGeometry::default(),
                        },
                    ),
                    (
                        11,
                        SnapshotNode {
                            parent: Some(10),
                            geometry: ElementGeometry::default(),
                        },
                    ),
                ]),
                elements_by_id: BTreeMap::from([
                    ("head-node".to_owned(), 9),
                    (child_id.to_owned(), 11),
                ]),
                body: Some(10),
            }
        }

        let replacement = snapshot("new");
        let mut backend =
            LayoutSnapshotBackend::new(snapshot("old"), move |_| Ok(replacement.clone()));
        let body = backend.query_selector("body").unwrap().unwrap();
        let head_node = backend.get_element_by_id("head-node").unwrap().unwrap();
        let old_child = backend.get_element_by_id("old").unwrap().unwrap();

        backend
            .set_inner_html(body, "<div id='new'></div>")
            .unwrap();

        let new_child = backend.get_element_by_id("new").unwrap().unwrap();
        assert_ne!(old_child, new_child, "source NodeIds are document-local");
        assert_eq!(backend.query_selector("body").unwrap(), Some(body));
        assert_eq!(
            backend.get_element_by_id("head-node").unwrap(),
            Some(head_node)
        );
        assert!(backend.bounding_client_rect(old_child).is_err());
    }

    #[test]
    fn layout_snapshot_preserves_hidden_geometry_parent_links_and_replacement_identity() {
        let wpt_root = tempfile::tempdir().unwrap();
        write_test_page(
            wpt_root.path(),
            "hidden.html",
            r#"<!doctype html><html><head>
                <style>#hidden { display: none }</style>
                <script src="/resources/testharness.js"></script>
            </head><body><div id="hidden">hidden</div><script>
                test(function() {
                    var hidden = document.getElementById('hidden');
                    assert_true(hidden !== null, 'hidden element remains in the DOM');
                    assert_approx_equals(hidden.offsetHeight, 0, 0);
                    var rect = hidden.getBoundingClientRect();
                    assert_approx_equals(rect.left, 0, 0);
                    assert_approx_equals(rect.top, 0, 0);
                    assert_approx_equals(rect.right, 0, 0);
                    assert_approx_equals(rect.bottom, 0, 0);
                    assert_approx_equals(rect.width, 0, 0);
                    assert_approx_equals(rect.height, 0, 0);
                    assert_true(document.body.getBoundingClientRect().width > 0,
                        'body geometry comes from layout');
                }, 'hidden geometry');
            </script></body></html>"#,
        );
        write_test_page(
            wpt_root.path(),
            "parent.html",
            r#"<!doctype html><html><head>
                <script src="/resources/testharness.js"></script>
            </head><body><div id="old-wrapper"><span id="old-target">old</span></div><script>
                var bodyBeforeReplacement = document.body;
                var oldTarget = document.getElementById('old-target');
                document.body.innerHTML = '<div id="wrapper"><span id="target">text</span></div>';
                setup({explicit_done: true});
                document.fonts.ready.then(function() {
                    test(function() {
                        var target = document.getElementById('target');
                        assert_true(bodyBeforeReplacement === document.body,
                            'body retains identity after innerHTML replacement');
                        assert_true(oldTarget !== target,
                            'a reparsed node gets a fresh wrapper even when source NodeIds repeat');
                        target.parentNode.style.display = 'none';
                        assert_true(document.getElementById('wrapper').style.display === 'none',
                            'parentNode refers to the real parent element');
                    }, 'parent links and snapshot identity');
                    done();
                });
            </script></body></html>"#,
        );

        let results = run_css_text_i18n(wpt_root.path()).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| result.error.is_none()));
        assert!(results.iter().all(TestHarnessFileResult::all_passed));
        assert_eq!(results[0].total(), 1);
        assert_eq!(results[1].total(), 1);
    }

    #[test]
    fn missing_and_empty_test_roots_report_distinct_errors() {
        let wpt_root = tempfile::tempdir().unwrap();
        assert!(matches!(
            run_css_text_i18n(&wpt_root.path().join("missing")),
            Err(TestHarnessRunError::Io(_))
        ));
        fs::create_dir_all(wpt_root.path().join(TEST_DIR)).unwrap();
        assert!(matches!(
            run_css_text_i18n(wpt_root.path()),
            Err(TestHarnessRunError::NoTests)
        ));
    }

    #[test]
    fn unreadable_test_file_is_reported_as_an_execution_error() {
        let wpt_root = tempfile::tempdir().unwrap();
        let result = run_testharness_file(&wpt_root.path().join("missing.html"), wpt_root.path());
        assert!(result.outcomes.is_empty());
        assert!(result.error.unwrap().starts_with("read test HTML:"));
    }

    #[test]
    #[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
    fn static_and_dynamic_i18n_fixtures_execute_all_tests() {
        let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
        for (relative, expected_count) in [
            (
                "css/css-text/i18n/css3-text-line-break-baspglwj-001.html",
                4,
            ),
            (
                "css/css-text/i18n/zh/css-text-line-break-zh-pr-normal.html",
                8,
            ),
        ] {
            let result = run_testharness_file(&wpt_root.join(relative), &wpt_root);
            assert!(result.error.is_none(), "{}: {:?}", relative, result.error);
            assert_eq!(result.total(), expected_count, "{relative}");
        }
    }
}
