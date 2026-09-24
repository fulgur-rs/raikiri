//! Executes the testharness-only tests in `css/css-text/i18n` against Raikiri.
//!
//! Reftest files in the same directory remain on the visual runner. This
//! module covers testharness scripts that query DOM geometry.

use std::fs;
use std::path::{Path, PathBuf};

use raikiri_js::TestOutcome;
use raikiri_js::dom::{DomBackend, DomNodeId, DomRect, ElementGeometry};
use raikiri_js::testharness::run_testharness_script;

use crate::reftest::{DEFAULT_REFTTEST_HEIGHT, DEFAULT_REFTTEST_WIDTH, prepare_wpt_live_document};

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

#[derive(Default)]
struct RawStyleDeclarationParser;

impl<'i> cssparser::DeclarationParser<'i> for RawStyleDeclarationParser {
    type Declaration = (String, String);
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: cssparser::CowRcStr<'i>,
        input: &mut cssparser::Parser<'i, 't>,
        _declaration_start: &cssparser::ParserState,
    ) -> Result<Self::Declaration, cssparser::ParseError<'i, Self::Error>> {
        let start = input.position();
        while input.next_including_whitespace_and_comments().is_ok() {}
        let value = input.slice(start..input.position()).trim().to_owned();
        Ok((name.to_string(), value))
    }
}

impl<'i> cssparser::AtRuleParser<'i> for RawStyleDeclarationParser {
    type Prelude = ();
    type AtRule = (String, String);
    type Error = ();
}

impl<'i> cssparser::QualifiedRuleParser<'i> for RawStyleDeclarationParser {
    type Prelude = ();
    type QualifiedRule = (String, String);
    type Error = ();
}

impl<'i> cssparser::RuleBodyItemParser<'i, (String, String), ()> for RawStyleDeclarationParser {
    fn parse_qualified(&self) -> bool {
        false
    }

    fn parse_declarations(&self) -> bool {
        true
    }
}

fn parse_inline_style(source: &str) -> Vec<(String, String)> {
    let mut input = cssparser::ParserInput::new(source);
    let mut parser = cssparser::Parser::new(&mut input);
    let mut declaration_parser = RawStyleDeclarationParser;
    cssparser::RuleBodyParser::new(&mut parser, &mut declaration_parser)
        .flatten()
        .map(|(name, value)| (name.to_string(), value))
        .collect()
}

fn strip_important(value: &str) -> &str {
    let trimmed = value.trim_end();
    let lower = trimmed.to_ascii_lowercase();
    lower
        .rfind("!important")
        .filter(|&start| lower[start + "!important".len()..].trim().is_empty())
        .map_or(trimmed, |start| trimmed[..start].trim_end())
}

/// Live bindings over one arena-owned Raikiri document. Node indices remain
/// stable because Document appends new arena nodes and never recycles slots.
struct LiveDocumentBackend {
    setup: crate::reftest::LiveWptSetup,
    wpt_root: PathBuf,
    page_scene: Option<raikiri::PageScene>,
    layout_dirty: bool,
}

impl LiveDocumentBackend {
    fn new(setup: crate::reftest::LiveWptSetup, wpt_root: &Path) -> Self {
        Self {
            setup,
            wpt_root: wpt_root.to_path_buf(),
            page_scene: None,
            layout_dirty: true,
        }
    }

    fn node_index(&self, handle: DomNodeId) -> Result<usize, String> {
        let index = usize::try_from(handle)
            .map_err(|_| format!("DOM node handle {handle} does not fit this target"))?;
        self.setup
            .uncascaded
            .dom
            .get_node(index)
            .map(|_| index)
            .ok_or_else(|| format!("stale DOM node handle {handle}"))
    }

    fn element_index(&self, handle: DomNodeId) -> Result<usize, String> {
        let index = self.node_index(handle)?;
        self.setup
            .uncascaded
            .dom
            .get_node(index)
            .and_then(|node| node.tag_name())
            .map(|_| index)
            .ok_or_else(|| format!("DOM node handle {handle} is not an element"))
    }

    fn handle_for(index: usize) -> Result<DomNodeId, String> {
        u64::try_from(index).map_err(|_| format!("DOM node index {index} exceeds u64"))
    }

    fn find_element(&self, mut predicate: impl FnMut(&raikiri_dom::Node) -> bool) -> Option<usize> {
        let document = &self.setup.uncascaded.dom;
        let mut pending = vec![document.root_index()];
        while let Some(index) = pending.pop() {
            let node = document.get_node(index)?;
            if node.tag_name().is_some() && predicate(node) {
                return Some(index);
            }
            pending.extend(node.children.iter().rev().copied());
        }
        None
    }

    fn find_id(&self, id: &str) -> Option<usize> {
        if id.is_empty() {
            return None;
        }
        self.find_element(|node| node.attribute("id") == Some(id))
    }

    fn matches_simple_selector(node: &raikiri_dom::Node, selector: &str) -> bool {
        if selector == "*" {
            return node.tag_name().is_some();
        }
        if let Some(id) = selector.strip_prefix('#') {
            return node.attribute("id") == Some(id);
        }
        if let Some(class) = selector.strip_prefix('.') {
            return node
                .attribute("class")
                .is_some_and(|classes| classes.split_ascii_whitespace().any(|item| item == class));
        }
        if let Some(attribute) = selector.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if let Some((name, value)) = attribute.split_once('=') {
                let value = value.trim().trim_matches(['\'', '"']);
                return node.attribute(name.trim()) == Some(value);
            }
            return node.attribute(attribute.trim()).is_some();
        }
        node.tag_name()
            .is_some_and(|tag| tag.eq_ignore_ascii_case(selector))
    }

    fn style_property_for(&self, index: usize, property: &str) -> String {
        let Some(style) = self
            .setup
            .uncascaded
            .dom
            .get_node(index)
            .and_then(|node| node.attribute("style"))
        else {
            return String::new();
        };
        parse_inline_style(style)
            .into_iter()
            .rev()
            .find(|(name, _)| {
                if property.starts_with("--") {
                    name == property
                } else {
                    name.eq_ignore_ascii_case(property)
                }
            })
            .map(|(_, value)| strip_important(&value).trim().to_owned())
            .unwrap_or_default()
    }

    fn invalidate_layout(&mut self) {
        self.layout_dirty = true;
        self.page_scene = None;
    }

    fn flush_layout(&mut self) -> Result<(), String> {
        if !self.layout_dirty {
            return Ok(());
        }
        self.setup.uncascaded.dom.mark_in_document_flags();
        let mut cascade = raikiri::build_cascaded_with_media_context_for_page(
            &self.setup.uncascaded,
            &self.setup.media_context,
            &self.setup.page_query,
        );
        let mut font_context = self.setup.font_context.clone();
        raikiri_dom::expand_font_face_aliases(
            &mut cascade.computed,
            self.setup.font_face_tree.font_faces(),
            &mut font_context,
        );
        let image_resolver = raikiri_net::ImageResolver::new(raikiri_net::FileNetworkProvider);
        raikiri_dom::layout_single_page_with_resolver_and_base_url(
            &mut self.setup.uncascaded.dom,
            &cascade,
            self.setup.page_box,
            font_context,
            &image_resolver,
            self.setup.document_base_url.as_ref(),
        )
        .map_err(|error| format!("live DOM layout: {error:?}"))?;

        // The engine's page scene clips fragments to its page box. CSSOM rect
        // reads must still work for offscreen nodes, so only the geometry
        // projection uses a tall box; layout itself used the configured WPT
        // viewport above.
        let mut projection_box = self.setup.page_box;
        projection_box.height = projection_box.height.max(1_000_000.0);
        let page_name = self
            .setup
            .page_query
            .page_name
            .as_ref()
            .map(ToString::to_string);
        self.page_scene = Some(raikiri::build_page_scene_for_page_named(
            &self.setup.uncascaded.dom,
            &cascade,
            projection_box,
            0,
            0.0,
            page_name,
        ));
        self.layout_dirty = false;
        Ok(())
    }

    fn geometry(&mut self, handle: DomNodeId) -> Result<ElementGeometry, String> {
        let index = self.element_index(handle)?;
        self.flush_layout()?;
        let node_id = raikiri_traits::NodeId::new(Self::handle_for(index)?);
        let Some(fragment) = self
            .page_scene
            .as_ref()
            .and_then(|scene| scene.fragments.get(&node_id))
            .and_then(|fragments| fragments.first())
        else {
            // A valid element with no rendered box (for example, display:none)
            // has the CSSOM zero rectangle rather than a backend error.
            return Ok(ElementGeometry::default());
        };
        let scene = self.page_scene.as_ref().expect("set by flush_layout");
        let (width, height) = scene
            .drawables
            .block_styles
            .get(&node_id)
            .and_then(|entry| entry.layout_size)
            .unwrap_or((fragment.width, fragment.height));
        let left = f64::from(fragment.x + scene.body_offset_pt.0);
        let top = f64::from(fragment.y + scene.body_offset_pt.1);
        let width = f64::from(width);
        let height = f64::from(height);
        Ok(ElementGeometry {
            offset_height: height.round(),
            bounding_client_rect: DomRect {
                left,
                top,
                right: left + width,
                bottom: top + height,
                width,
                height,
            },
        })
    }
}

impl DomBackend for LiveDocumentBackend {
    fn get_element_by_id(&mut self, id: &str) -> Result<Option<DomNodeId>, String> {
        self.find_id(id).map(Self::handle_for).transpose()
    }

    fn query_selector(&mut self, selector: &str) -> Result<Option<DomNodeId>, String> {
        let selector = selector.trim();
        self.find_element(|node| Self::matches_simple_selector(node, selector))
            .map(Self::handle_for)
            .transpose()
    }

    fn parent_node(&mut self, node: DomNodeId) -> Result<Option<DomNodeId>, String> {
        let index = self.element_index(node)?;
        let Some(parent) = self.setup.uncascaded.dom.parent_of(index) else {
            return Ok(None);
        };
        let Some(parent_node) = self.setup.uncascaded.dom.get_node(parent) else {
            return Ok(None); // cov:ignore: parent_of returns only existing append-only arena indices.
        };
        // The facade has no Document or DocumentFragment wrapper.
        if parent_node.tag_name().is_none() {
            return Ok(None);
        }
        Self::handle_for(parent).map(Some)
    }

    fn offset_height(&mut self, node: DomNodeId) -> Result<f64, String> {
        Ok(self.geometry(node)?.offset_height)
    }

    fn bounding_client_rect(&mut self, node: DomNodeId) -> Result<DomRect, String> {
        Ok(self.geometry(node)?.bounding_client_rect)
    }

    fn inner_html(&mut self, node: DomNodeId) -> Result<String, String> {
        let index = self.element_index(node)?;
        self.setup
            .uncascaded
            .dom
            .serialize_inner_html(index)
            .map_err(|error| format!("serialize innerHTML: {error}"))
    }

    fn set_inner_html(&mut self, node: DomNodeId, value: &str) -> Result<(), String> {
        let target = self.element_index(node)?;
        let target_node = self
            .setup
            .uncascaded
            .dom
            .get_node(target)
            .ok_or_else(|| format!("innerHTML target {node} is out of range"))?;
        let context_tag = target_node
            .tag_name()
            .ok_or_else(|| format!("innerHTML target {node} is not an element"))?
            .to_owned();
        let context_namespace = self
            .setup
            .uncascaded
            .dom
            .element_namespace_uri(target)
            .ok_or_else(|| format!("innerHTML target {node} has no element namespace"))?
            .to_owned();
        let removed_sources = crate::reftest::live_wpt_stylesheet_sources_in_subtree(
            &self.setup.uncascaded.dom,
            target,
        );
        let fragment = crate::reftest::parse_wpt_inner_html_fragment(
            value,
            &context_tag,
            &context_namespace,
            self.setup.page_box.width as u32,
            self.setup.page_box.height as u32,
            self.setup.document_base_url.as_ref(),
            &self.wpt_root,
        )?; // cov:ignore: UTF-8 markup is parsed from memory; its reader and parser recover without I/O/encoding errors.
        let source_parent = fragment.dom.root_index();
        let mut added_sources = fragment.stylesheet_sources.clone();
        if context_namespace == "http://www.w3.org/1999/xhtml"
            && context_tag.eq_ignore_ascii_case("template")
        {
            // Template contents are inert and must not add active document CSS.
            added_sources.clear();
        } else if context_namespace == "http://www.w3.org/1999/xhtml"
            && context_tag.eq_ignore_ascii_case("style")
            && added_sources.is_empty()
            && let Some(root) = fragment.dom.get_node(source_parent)
        {
            let source = root
                .children
                .iter()
                .filter_map(|&child| fragment.dom.get_node(child)?.text_content())
                .collect::<String>();
            if !source.is_empty() {
                added_sources.push(source);
            }
        }
        self.setup
            .uncascaded
            .dom
            .replace_children_from(target, &fragment.dom, source_parent);
        crate::reftest::update_live_wpt_stylesheet_sources(
            &mut self.setup,
            removed_sources,
            added_sources,
            &self.wpt_root,
        );
        self.invalidate_layout();
        Ok(())
    }

    fn get_attribute(&mut self, node: DomNodeId, name: &str) -> Result<Option<String>, String> {
        let index = self.element_index(node)?;
        Ok(self
            .setup
            .uncascaded
            .dom
            .element_attribute(index, name)
            .map(str::to_owned))
    }

    fn has_attribute(&mut self, node: DomNodeId, name: &str) -> Result<bool, String> {
        Ok(self.get_attribute(node, name)?.is_some())
    }

    fn set_attribute(&mut self, node: DomNodeId, name: &str, value: &str) -> Result<(), String> {
        let index = self.element_index(node)?;
        self.setup
            .uncascaded
            .dom
            .set_element_attribute(index, name, value)?;
        self.invalidate_layout();
        Ok(())
    }

    fn remove_attribute(&mut self, node: DomNodeId, name: &str) -> Result<(), String> {
        let index = self.element_index(node)?;
        self.setup
            .uncascaded
            .dom
            .remove_element_attribute(index, name)?;
        self.invalidate_layout();
        Ok(())
    }

    fn style_property(&mut self, node: DomNodeId, property: &str) -> Result<String, String> {
        let index = self.element_index(node)?;
        Ok(self.style_property_for(index, property))
    }

    fn set_style_property(
        &mut self,
        node: DomNodeId,
        property: &str,
        value: &str,
    ) -> Result<(), String> {
        let index = self.element_index(node)?;
        let property = property.trim();
        if property.is_empty() {
            return Ok(());
        }
        let mut declarations = self
            .setup
            .uncascaded
            .dom
            .get_node(index)
            .and_then(|element| element.attribute("style"))
            .map(parse_inline_style)
            .unwrap_or_default();
        declarations.retain(|(name, _)| {
            if property.starts_with("--") {
                name != property
            } else {
                !name.eq_ignore_ascii_case(property)
            }
        });
        if !value.trim().is_empty() {
            declarations.push((property.to_owned(), value.to_owned()));
        }
        let serialized = declarations
            .into_iter()
            .map(|(name, value)| format!("{name}: {value};"))
            .collect::<Vec<_>>()
            .join(" ");
        self.setup
            .uncascaded
            .dom
            .set_element_inline_style(index, Some(serialized.into()));
        self.invalidate_layout();
        Ok(())
    }
}

/// Run every testharness-only HTML page under `css/css-text/i18n`.
///
/// Each script runs against a live Raikiri document. DOM and style writes
/// update arena nodes; geometry reads lazily recascade and relayout that same
/// document before returning current page-scene fragments.
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

    let page_base = path.parent().unwrap_or(wpt_root);
    let setup = match prepare_wpt_live_document(
        &html,
        DEFAULT_REFTTEST_WIDTH,
        DEFAULT_REFTTEST_HEIGHT,
        page_base,
        wpt_root,
    ) {
        Ok(setup) => setup,
        // cov:ignore: prepare_wpt_live_document parses in-memory valid UTF-8 and HTML parsing recovers from markup errors.
        Err(error) => {
            return TestHarnessFileResult {
                test_id,
                outcomes: Vec::new(),
                error: Some(format!("prepare live Raikiri document: {error}")),
            };
        }
    };

    let backend = LiveDocumentBackend::new(setup, wpt_root);
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

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_BY_THREE_PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 3, 8, 6,
        0, 0, 0, 185, 234, 222, 129, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240,
        31, 10, 209, 24, 0, 158, 124, 11, 245, 228, 127, 198, 52, 0, 0, 0, 0, 73, 69, 78, 68, 174,
        66, 96, 130,
    ];

    fn live_backend(html: &str, root: &Path) -> LiveDocumentBackend {
        let setup = crate::reftest::prepare_wpt_live_document(html, 800, 600, root, root)
            .expect("valid test HTML should configure the live document");
        LiveDocumentBackend::new(setup, root)
    }

    #[test]
    fn live_selectors_and_inline_style_access_cover_dom_facade_queries() {
        let root = tempfile::tempdir().unwrap();
        let html = r#"<!doctype html><html><body>
            <div id="target" class="first second" data-key="value"
                 style="color: red; --token: first"></div>
            <span id="unstyled"></span>
        </body></html>"#;
        let mut backend = live_backend(html, root.path());
        let target = backend.get_element_by_id("target").unwrap().unwrap();
        let unstyled = backend.get_element_by_id("unstyled").unwrap().unwrap();
        let body = backend.query_selector("body").unwrap().unwrap();
        let html_element = backend.query_selector("html").unwrap().unwrap();

        assert!(backend.get_element_by_id("").unwrap().is_none());
        assert!(backend.query_selector("*").unwrap().is_some());
        assert_eq!(backend.query_selector("#target").unwrap(), Some(target));
        assert_eq!(backend.query_selector(".second").unwrap(), Some(target));
        assert_eq!(
            backend.query_selector("[data-key='value']").unwrap(),
            Some(target)
        );
        assert_eq!(backend.query_selector("[data-key]").unwrap(), Some(target));
        assert_eq!(backend.query_selector("DIV").unwrap(), Some(target));
        assert_eq!(backend.parent_node(target).unwrap(), Some(body));
        assert_eq!(backend.parent_node(html_element).unwrap(), None);

        assert_eq!(backend.style_property(target, "COLOR").unwrap(), "red");
        assert_eq!(backend.style_property(target, "--token").unwrap(), "first");
        assert_eq!(backend.style_property(unstyled, "color").unwrap(), "");
        backend.set_style_property(target, "", "ignored").unwrap();
        backend
            .set_style_property(target, "--token", "second")
            .unwrap();
        assert_eq!(backend.style_property(target, "--token").unwrap(), "second");
        backend.set_style_property(target, "color", "").unwrap();
        assert_eq!(backend.style_property(target, "color").unwrap(), "");
    }

    #[test]
    fn setting_style_element_inner_html_reloads_stylesheet_and_removal() {
        let root = tempfile::tempdir().unwrap();
        let html = r#"<!doctype html><html><head><style id="dynamic"></style></head>
            <body><div id="probe"></div></body></html>"#;
        let mut backend = live_backend(html, root.path());
        let style = backend.get_element_by_id("dynamic").unwrap().unwrap();
        let probe = backend.get_element_by_id("probe").unwrap().unwrap();

        backend
            .set_inner_html(style, "#probe { width: 27px; height: 19px; }")
            .unwrap();
        let styled = backend.bounding_client_rect(probe).unwrap();
        assert_eq!(styled.width, 27.0);
        assert_eq!(styled.height, 19.0);

        backend.set_inner_html(style, "").unwrap();
        let unstyled = backend.bounding_client_rect(probe).unwrap();
        assert_ne!(unstyled.width, 27.0);
    }

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

    fn write_test_page(wpt_root: &Path, name: &str, html: &str) {
        let test_dir = wpt_root.join(TEST_DIR);
        fs::create_dir_all(&test_dir).unwrap();
        fs::write(test_dir.join(name), html).unwrap();
    }

    #[test]
    fn live_backend_updates_identity_attributes_style_and_current_geometry() {
        let wpt_root = tempfile::tempdir().unwrap();
        let html = r#"<!doctype html><html><head><style>
            @media screen { #media-box { width: 23px; height: 17px; } }
            @media print { #media-box { width: 230px; height: 170px; } }
            </style></head><body><p>initial</p></body></html>"#;
        let setup = prepare_wpt_live_document(
            html,
            DEFAULT_REFTTEST_WIDTH,
            DEFAULT_REFTTEST_HEIGHT,
            wpt_root.path(),
            wpt_root.path(),
        )
        .unwrap();
        let backend = LiveDocumentBackend::new(setup, wpt_root.path());

        raikiri_js::run_script(
            r#"
                var body = document.body;
                if (document.querySelector('body') !== body)
                    throw new Error('document/body wrappers are not stable');
                body.innerHTML = '<div id="box" style="width:10px;height:12px"></div>' +
                    '<div id="media-box"></div>' +
                    '<div id="container"><span id="child">old</span></div>';

                var mediaBox = document.getElementById('media-box');
                var mediaRect = mediaBox.getBoundingClientRect();
                if (mediaRect.width !== 23 || mediaRect.height !== 17)
                    throw new Error('live testharness layout did not use screen media');
                var box = document.getElementById('box');
                if (box !== document.querySelector('#box') || box.getAttribute('id') !== 'box')
                    throw new Error('repeated live element queries lost identity');
                box.setAttribute('data-empty', '');
                box.setAttribute('data-count', 7);
                if (!box.hasAttribute('data-empty') || box.getAttribute('data-empty') !== '' ||
                    box.getAttribute('data-count') !== '7')
                    throw new Error('attribute reads do not reflect mutations');
                box.setAttribute('DATA-Key', 'first');
                box.setAttribute('data-key', 'second');
                if (box.getAttribute('DATA-KEY') !== 'second')
                    throw new Error('HTML attribute names were not case-insensitive');
                box.removeAttribute('data-count');
                if (box.hasAttribute('data-count') || box.getAttribute('data-count') !== null)
                    throw new Error('removeAttribute did not update the element');
                if (!body.innerHTML.includes('data-empty=""'))
                    throw new Error('innerHTML did not serialize current attributes');

                var child = document.getElementById('child');
                var container = document.querySelector('#container');
                if (child.parentNode !== container)
                    throw new Error('parentNode does not follow the live tree');

                box.style.width = '20px';
                var before = box.getBoundingClientRect();
                box.style.width = '40px';
                box.style.height = '32px';
                var after = box.getBoundingClientRect();
                if (box.style.getPropertyValue('width') !== '40px')
                    throw new Error('CSSStyleDeclaration read missed the inline write');
                if (!(after.width > before.width && after.height > before.height &&
                    box.offsetHeight > 20))
                    throw new Error('geometry did not reflect current inline style');

                var oldChild = child;
                var oldContainer = oldChild.parentNode;
                body.innerHTML = '<div id="replacement"><span id="new-child">new</span></div>';
                if (document.body !== body || oldChild.parentNode !== oldContainer ||
                    oldContainer.parentNode !== null || document.getElementById('child') !== null)
                    throw new Error('innerHTML did not replace the actual body children');
                var newChild = document.getElementById('new-child');
                if (newChild === oldChild || newChild.parentNode !== document.getElementById('replacement'))
                    throw new Error('replacement nodes reused old identities or parent links');

                body.innerHTML = '<table><tbody id="rows"></tbody></table>' +
                    '<template id="template"></template><svg id="svg"></svg>';
                var rows = document.getElementById('rows');
                rows.innerHTML = '<tr><td id="cell">cell</td></tr>';
                var cell = document.getElementById('cell');
                if (cell === null || cell.parentNode === null ||
                    cell.parentNode.parentNode !== rows)
                    throw new Error('table-context innerHTML did not preserve fragment semantics');
                var template = document.getElementById('template');
                template.innerHTML = '<span id="inert-template-child">inside</span>';
                if (!template.innerHTML.includes('inert-template-child') ||
                    document.getElementById('inert-template-child') !== null)
                    throw new Error('template innerHTML did not target inert template contents');
                var svg = document.getElementById('svg');
                svg.innerHTML = '<g id="svg-child" viewBox="0 0 1 1"></g>';
                var svgChild = document.getElementById('svg-child');
                if (svgChild.getAttribute('viewBox') !== '0 0 1 1' ||
                    svgChild.getAttribute('viewbox') !== null)
                    throw new Error('SVG innerHTML or attribute-name case was incorrect');

                body.innerHTML = '<style>#from-style { display: none }</style>' +
                    '<div id="from-style">hidden</div>';
                if (document.getElementById('from-style').offsetHeight !== 0)
                    throw new Error('a style element inserted by innerHTML did not reach cascade');
                body.innerHTML = '<div id="from-style">shown</div>';
                if (document.getElementById('from-style').offsetHeight <= 0)
                    throw new Error('a removed style element still affected cascade');
            "#,
            backend,
        )
        .unwrap();
    }

    #[test]
    fn live_document_preserves_hidden_geometry_parent_links_and_replacement_identity() {
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
                    }, 'parent links and live node identity');
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
    fn live_testharness_resolves_relative_stylesheets_and_images_from_the_page_directory() {
        let wpt_root = tempfile::tempdir().unwrap();
        let test_dir = wpt_root.path().join(TEST_DIR).join("nested");
        fs::create_dir_all(&test_dir).unwrap();
        fs::write(
            test_dir.join("relative.css"),
            "#relative { width: 27px; height: 19px; }",
        )
        .unwrap();
        fs::write(test_dir.join("small.png"), TWO_BY_THREE_PNG).unwrap();
        let html = r#"<!doctype html>
            <html><head>
              <link rel="stylesheet" href="relative.css">
              <script src="/resources/testharness.js"></script>
            </head><body>
              <div id="relative"></div>
              <img id="pixel" src="small.png">
              <script>
                test(function () {
                  assert_approx_equals(
                    document.getElementById('relative').getBoundingClientRect().width,
                    27, 0.1);
                  assert_approx_equals(document.getElementById('pixel').offsetHeight, 3, 0.1);
                }, 'relative stylesheets and images use the test page directory');
              </script>
            </body></html>"#;
        let test_file = test_dir.join("relative-resources.html");
        fs::write(&test_file, html).unwrap();

        let result = run_testharness_file(&test_file, wpt_root.path());
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.total(), 1);
        assert!(result.all_passed(), "{:?}", result.outcomes);
    }

    // cov:ignore: this fetched-WPT fixture test runs in the gate's explicit --ignored pass, not the coverage pass.
    #[test]
    #[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
    fn static_and_dynamic_i18n_fixtures_report_assertion_outcomes() {
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
            assert!(
                !result.outcomes.is_empty(),
                "{relative} produced no assertions"
            );
            if relative == "css/css-text/i18n/css3-text-line-break-baspglwj-001.html" {
                assert!(result.all_passed(), "{relative}: {:?}", result.outcomes);
            }
            // The dynamic fixture's remaining failures are engine-level line-break
            // gaps, not runner execution errors; this test verifies those outcomes
            // are returned without promoting them to a DOM-binding pass gate.
        }
    }
}
