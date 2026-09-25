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
    computed_styles: Option<Vec<raikiri_style::ComputedValues>>,
    layout_dirty: bool,
    #[cfg(test)]
    layout_flush_count: usize,
}

impl LiveDocumentBackend {
    fn new(setup: crate::reftest::LiveWptSetup, wpt_root: &Path) -> Self {
        Self {
            setup,
            wpt_root: wpt_root.to_path_buf(),
            page_scene: None,
            computed_styles: None,
            layout_dirty: true,
            #[cfg(test)]
            layout_flush_count: 0,
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

    fn is_connected(&self, index: usize) -> bool {
        let document = &self.setup.uncascaded.dom;
        let mut pending = vec![document.root_index()];
        let mut visited = std::collections::HashSet::new();
        while let Some(current) = pending.pop() {
            if current == index {
                return true;
            }
            if visited.insert(current)
                && let Some(node) = document.get_node(current)
            {
                pending.extend(node.children.iter().copied());
            }
        }
        false
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
        self.computed_styles = None;
    }

    fn flush_layout(&mut self) -> Result<(), String> {
        if !self.layout_dirty {
            return Ok(());
        }
        #[cfg(test)]
        {
            self.layout_flush_count += 1;
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
        self.computed_styles = Some(cascade.computed);
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

    fn create_element(&mut self, local_name: &str) -> Result<DomNodeId, String> {
        let local_name = local_name.to_ascii_lowercase();
        let index = self
            .setup
            .uncascaded
            .dom
            .create_detached_element(&local_name)?;
        self.invalidate_layout();
        Self::handle_for(index)
    }

    fn append_child(&mut self, parent: DomNodeId, child: DomNodeId) -> Result<(), String> {
        let parent = self.element_index(parent)?;
        let child = self.node_index(child)?;
        let child_was_connected = self.is_connected(child);
        let parent_is_connected = self.is_connected(parent);
        let moved_stylesheets = (child_was_connected != parent_is_connected).then(|| {
            crate::reftest::live_wpt_stylesheet_sources_in_subtree(
                &self.setup.uncascaded.dom,
                child,
            )
        });
        self.setup.uncascaded.dom.append_child(parent, child)?;
        if let Some(sources) = moved_stylesheets {
            let (removed, added) = if child_was_connected {
                (sources, Vec::new())
            } else {
                (Vec::new(), sources)
            };
            crate::reftest::update_live_wpt_stylesheet_sources(
                &mut self.setup,
                removed,
                added,
                &self.wpt_root,
            );
        } // cov:ignore: llvm-cov emits no hit count for this block-closing brace.
        self.invalidate_layout();
        Ok(())
    }

    fn text_content(&mut self, node: DomNodeId) -> Result<String, String> {
        let index = self.element_index(node)?;
        self.setup
            .uncascaded
            .dom
            .element_text_content(index)
            .ok_or_else(|| format!("DOM node handle {node} has no element textContent"))
    }

    fn set_text_content(&mut self, node: DomNodeId, value: &str) -> Result<(), String> {
        let index = self.element_index(node)?;
        let connected = self.is_connected(index);
        let removed_sources = if connected {
            crate::reftest::live_wpt_stylesheet_sources_in_subtree(
                &self.setup.uncascaded.dom,
                index,
            )
        } else {
            Vec::new()
        };
        self.setup
            .uncascaded
            .dom
            .set_element_text_content(index, value)?;
        if connected {
            let added_sources = crate::reftest::live_wpt_stylesheet_sources_in_subtree(
                &self.setup.uncascaded.dom,
                index,
            );
            crate::reftest::update_live_wpt_stylesheet_sources(
                &mut self.setup,
                removed_sources,
                added_sources,
                &self.wpt_root,
            );
        }
        self.invalidate_layout();
        Ok(())
    }

    fn style_property(&mut self, node: DomNodeId, property: &str) -> Result<String, String> {
        let index = self.element_index(node)?;
        Ok(self.style_property_for(index, property))
    }

    fn computed_style_property(
        &mut self,
        node: DomNodeId,
        property: &str,
    ) -> Result<Option<String>, String> {
        // Unsupported properties bail out before the layout flush below.
        cssparser::match_ignore_ascii_case! { property,
            "white-space"
            | "white-space-collapse"
            | "line-break"
            | "hyphenate-character"
            | "hyphenate-limit-chars"
            | "hyphens"
            | "font-kerning"
            | "font-variant-caps"
            | "font-optical-sizing"
            | "font-variant-emoji"
            | "font-language-override"
            | "font-variant-ligatures"
            | "font-synthesis"
            | "font-variant-position"
            | "font-palette"
            | "font-variant-numeric"
            | "font-variant-east-asian"
            | "overflow-wrap"
            | "word-wrap"
            | "word-break"
            | "text-transform"
            | "text-combine-upright"
            | "text-orientation"
            | "direction"
            | "unicode-bidi"
            | "writing-mode"
            | "text-autospace"
            | "word-space-transform"
            | "text-decoration-skip-ink"
            | "text-decoration-skip-spaces"
            | "text-decoration"
            | "text-decoration-style"
            | "text-decoration-line"
            | "text-shadow"
            | "text-decoration-inset"
            | "text-decoration-color"
            | "text-underline-position"
            | "text-underline-offset"
            | "text-emphasis-position"
            | "text-emphasis-style"
            | "text-emphasis"
            | "text-spacing-trim"
            | "text-spacing"
            | "text-wrap"
            | "text-wrap-mode"
            | "text-wrap-style"
            | "text-align"
            | "text-align-last"
            | "text-justify"
            | "text-indent"
            | "tab-size"
            | "letter-spacing"
            | "word-spacing" => {},
            _ => return Ok(None),
        }
        let index = self.element_index(node)?;
        self.flush_layout()?;
        let computed = self
            .computed_styles
            .as_ref()
            .and_then(|styles| styles.get(index))
            .ok_or_else(|| format!("computed style is missing for DOM node {node}"))?;
        if property.eq_ignore_ascii_case("direction") {
            let value = match computed.direction {
                raikiri_style::property::Direction::Ltr => "ltr",
                raikiri_style::property::Direction::Rtl => "rtl",
                _ => return Ok(None),
            };
            return Ok(Some(value.to_owned()));
        }
        if property.eq_ignore_ascii_case("font-kerning") {
            let property_value =
                raikiri_style::property::PropertyValue::FontKerning(computed.font_kerning);
            return Ok(raikiri_style::property::serialize_value(&property_value));
        }
        if property.eq_ignore_ascii_case("font-variant-caps") {
            let value = match computed.font_variant_caps {
                raikiri_style::property::FontVariantCaps::Normal => "normal",
                raikiri_style::property::FontVariantCaps::SmallCaps => "small-caps",
                raikiri_style::property::FontVariantCaps::AllSmallCaps => "all-small-caps",
                raikiri_style::property::FontVariantCaps::PetiteCaps => "petite-caps",
                raikiri_style::property::FontVariantCaps::AllPetiteCaps => "all-petite-caps",
                raikiri_style::property::FontVariantCaps::Unicase => "unicase",
                raikiri_style::property::FontVariantCaps::TitlingCaps => "titling-caps",
                _ => return Ok(None),
            };
            return Ok(Some(value.to_owned()));
        }
        if property.eq_ignore_ascii_case("font-optical-sizing") {
            let property_value = raikiri_style::property::PropertyValue::FontOpticalSizing(
                computed.font_optical_sizing,
            );
            return Ok(raikiri_style::property::serialize_value(&property_value));
        }
        if property.eq_ignore_ascii_case("font-variant-emoji") {
            let property_value = raikiri_style::property::PropertyValue::FontVariantEmoji(
                computed.font_variant_emoji,
            );
            return Ok(raikiri_style::property::serialize_value(&property_value));
        }
        if property.eq_ignore_ascii_case("font-language-override") {
            let property_value = raikiri_style::property::PropertyValue::FontLanguageOverride(
                computed.font_language_override.clone(),
            );
            return Ok(raikiri_style::property::serialize_value(&property_value));
        }
        if property.eq_ignore_ascii_case("font-variant-ligatures") {
            let property_value = raikiri_style::property::PropertyValue::FontVariantLigatures(
                computed.font_variant_ligatures,
            );
            return Ok(raikiri_style::property::serialize_value(&property_value));
        }
        if property.eq_ignore_ascii_case("font-synthesis") {
            let property_value =
                raikiri_style::property::PropertyValue::FontSynthesis(computed.font_synthesis);
            return Ok(raikiri_style::property::serialize_value(&property_value));
        }
        if property.eq_ignore_ascii_case("font-variant-position") {
            let property_value = raikiri_style::property::PropertyValue::FontVariantPosition(
                computed.font_variant_position,
            );
            return Ok(raikiri_style::property::serialize_value(&property_value));
        }
        if property.eq_ignore_ascii_case("font-palette") {
            let property_value =
                raikiri_style::property::PropertyValue::FontPalette(computed.font_palette.clone());
            return Ok(raikiri_style::property::serialize_value(&property_value));
        }
        if property.eq_ignore_ascii_case("font-variant-numeric") {
            let property_value = raikiri_style::property::PropertyValue::FontVariantNumeric(
                computed.font_variant_numeric,
            );
            return Ok(raikiri_style::property::serialize_value(&property_value));
        }
        if property.eq_ignore_ascii_case("font-variant-east-asian") {
            let property_value = raikiri_style::property::PropertyValue::FontVariantEastAsian(
                computed.font_variant_east_asian,
            );
            return Ok(raikiri_style::property::serialize_value(&property_value));
        }
        if property.eq_ignore_ascii_case("text-combine-upright") {
            let value = match computed.text_combine_upright {
                raikiri_style::property::TextCombineUpright::None => "none",
                raikiri_style::property::TextCombineUpright::All => "all",
                _ => return Ok(None),
            };
            return Ok(Some(value.to_owned()));
        }
        if property.eq_ignore_ascii_case("text-orientation") {
            let value = match computed.text_orientation {
                raikiri_style::property::TextOrientation::Mixed => "mixed",
                raikiri_style::property::TextOrientation::Upright => "upright",
                raikiri_style::property::TextOrientation::Sideways => "sideways",
                _ => return Ok(None),
            };
            return Ok(Some(value.to_owned()));
        }
        if property.eq_ignore_ascii_case("writing-mode") {
            let value = match computed.cssom_writing_mode {
                raikiri_style::property::WritingMode::HorizontalTb => "horizontal-tb",
                raikiri_style::property::WritingMode::VerticalRl => "vertical-rl",
                raikiri_style::property::WritingMode::VerticalLr => "vertical-lr",
                raikiri_style::property::WritingMode::SidewaysRl => "sideways-rl",
                raikiri_style::property::WritingMode::SidewaysLr => "sideways-lr",
                _ => return Ok(None),
            };
            return Ok(Some(value.to_owned()));
        }
        if property.eq_ignore_ascii_case("unicode-bidi") {
            let value = match computed.unicode_bidi {
                raikiri_style::property::UnicodeBidi::Normal => "normal",
                raikiri_style::property::UnicodeBidi::Embed => "embed",
                raikiri_style::property::UnicodeBidi::Isolate => "isolate",
                raikiri_style::property::UnicodeBidi::BidiOverride => "bidi-override",
                raikiri_style::property::UnicodeBidi::IsolateOverride => "isolate-override",
                raikiri_style::property::UnicodeBidi::Plaintext => "plaintext",
                _ => return Ok(None),
            };
            return Ok(Some(value.to_owned()));
        }
        if property.eq_ignore_ascii_case("text-wrap") {
            let mode = match computed.text_wrap {
                raikiri_style::property::TextWrapMode::Wrap => "wrap",
                raikiri_style::property::TextWrapMode::Nowrap => "nowrap",
                _ => return Ok(None),
            };
            let style = match computed.text_wrap_style {
                raikiri_style::property::TextWrapStyle::Auto => "auto",
                raikiri_style::property::TextWrapStyle::Balance => "balance",
                raikiri_style::property::TextWrapStyle::Pretty => "pretty",
                raikiri_style::property::TextWrapStyle::Stable => "stable",
                _ => return Ok(None),
            };
            let value = if mode == "wrap" {
                if style == "auto" {
                    "wrap".to_owned()
                } else {
                    style.to_owned()
                }
            } else if style == "auto" {
                mode.to_owned()
            } else {
                format!("{mode} {style}")
            };
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("hyphenate-limit-chars") {
            let value = &computed.hyphenate_limit_chars;
            let values = [&value.total, &value.before, &value.after];
            // Omit the third value only when it defaults to the second; two `auto` values serialize as `auto`.
            let value_count = if values[1] == values[2] {
                if matches!(
                    values[1],
                    &raikiri_style::property::HyphenateLimitCharsValue::Auto
                ) {
                    1
                } else {
                    2
                }
            } else {
                3
            };
            let mut serialized = Vec::with_capacity(value_count);
            for component in values.iter().take(value_count).copied() {
                serialized.push(match *component {
                    raikiri_style::property::HyphenateLimitCharsValue::Auto => "auto".to_owned(),
                    raikiri_style::property::HyphenateLimitCharsValue::Integer(value) => {
                        value.to_string()
                    }
                    _ => return Ok(None),
                });
            }
            return Ok(Some(serialized.join(" ")));
        }
        if property.eq_ignore_ascii_case("hyphenate-character") {
            let value = match &computed.hyphenate_character {
                raikiri_style::property::HyphenateCharacter::Auto => "auto".to_owned(),
                raikiri_style::property::HyphenateCharacter::String(value) => {
                    let mut serialized = String::new();
                    cssparser::serialize_string(value, &mut serialized)
                        .expect("serializing a CSS string into String cannot fail");
                    serialized
                }
                _ => return Ok(None),
            };
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("text-spacing") {
            let Some(trim) = raikiri_style::property::serialize_value(
                &raikiri_style::property::PropertyValue::TextSpacingTrim(
                    computed.text_spacing_trim,
                ),
            ) else {
                return Ok(None);
            };
            let Some(autospace) = raikiri_style::property::serialize_value(
                &raikiri_style::property::PropertyValue::TextAutospace(computed.text_autospace),
            ) else {
                return Ok(None);
            };
            let value = if trim == "normal" && autospace == "normal" {
                "normal".to_owned()
            } else if trim == "space-all" && autospace == "no-autospace" {
                "none".to_owned()
            } else if trim == "auto" && autospace == "auto" {
                "auto".to_owned()
            } else {
                // Keep `normal` explicit when omitting it would turn the other
                // `auto` longhand into the shorthand's two-longhand `auto` alias.
                let keep_normal = (trim == "auto") != (autospace == "auto");
                let mut components = Vec::with_capacity(2);
                if trim != "normal" || keep_normal {
                    components.push(trim);
                }
                if autospace != "normal" || keep_normal {
                    components.push(autospace);
                }
                components.join(" ")
            };
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("text-spacing-trim") {
            let value = match computed.text_spacing_trim {
                raikiri_style::property::TextSpacingTrim::Auto => "auto",
                raikiri_style::property::TextSpacingTrim::Normal => "normal",
                raikiri_style::property::TextSpacingTrim::SpaceAll => "space-all",
                raikiri_style::property::TextSpacingTrim::TrimBoth => "trim-both",
                raikiri_style::property::TextSpacingTrim::TrimAll => "trim-all",
                raikiri_style::property::TextSpacingTrim::TrimStart => "trim-start",
                raikiri_style::property::TextSpacingTrim::SpaceFirst => "space-first",
                _ => return Ok(None),
            };
            return Ok(Some(value.to_owned()));
        }
        if property.eq_ignore_ascii_case("text-autospace") {
            let value = match computed.text_autospace {
                raikiri_style::property::TextAutospace::Normal => "normal".to_owned(),
                raikiri_style::property::TextAutospace::Auto => "auto".to_owned(),
                raikiri_style::property::TextAutospace::NoAutospace => "no-autospace".to_owned(),
                raikiri_style::property::TextAutospace::Custom {
                    ideograph_alpha,
                    ideograph_numeric,
                    punctuation,
                    mode,
                } => {
                    let mut keywords = Vec::with_capacity(4);
                    if ideograph_alpha {
                        keywords.push("ideograph-alpha");
                    }
                    if ideograph_numeric {
                        keywords.push("ideograph-numeric");
                    }
                    if punctuation {
                        keywords.push("punctuation");
                    }
                    match mode {
                        raikiri_style::property::TextAutospaceMode::None => {}
                        raikiri_style::property::TextAutospaceMode::Insert => {
                            keywords.push("insert");
                        }
                        raikiri_style::property::TextAutospaceMode::Replace => {
                            keywords.push("replace");
                        }
                        _ => return Ok(None),
                    }
                    keywords.join(" ")
                }
                _ => return Ok(None),
            };
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("word-space-transform") {
            let value = match computed.word_space_transform {
                raikiri_style::property::WordSpaceTransform::None => "none",
                raikiri_style::property::WordSpaceTransform::Space => "space",
                raikiri_style::property::WordSpaceTransform::IdeographicSpace => {
                    "ideographic-space"
                }
                raikiri_style::property::WordSpaceTransform::SpaceAutoPhrase => "space auto-phrase",
                raikiri_style::property::WordSpaceTransform::IdeographicSpaceAutoPhrase => {
                    "ideographic-space auto-phrase"
                }
                _ => return Ok(None),
            };
            return Ok(Some(value.to_owned()));
        }
        if property.eq_ignore_ascii_case("text-decoration-skip-ink") {
            let Some(value) = raikiri_style::property::serialize_value(
                &raikiri_style::property::PropertyValue::TextDecorationSkipInk(
                    computed.text_decoration_skip_ink,
                ),
            ) else {
                return Ok(None);
            };
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("text-decoration-skip-spaces") {
            let Some(value) = raikiri_style::property::serialize_value(
                &raikiri_style::property::PropertyValue::TextDecorationSkipSpaces(
                    computed.text_decoration_skip_spaces,
                ),
            ) else {
                return Ok(None);
            };
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("text-decoration") {
            let mut components = Vec::with_capacity(4);
            let Some(line) = raikiri_style::property::serialize_value(
                &raikiri_style::property::PropertyValue::TextDecorationLine(
                    computed.text_decoration_line,
                ),
            ) else {
                return Ok(None);
            };
            if line != "none" {
                components.push(line);
            }
            if computed.text_decoration_style != raikiri_style::property::TextDecorationStyle::Solid
            {
                let Some(style) = raikiri_style::property::serialize_value(
                    &raikiri_style::property::PropertyValue::TextDecorationStyle(
                        computed.text_decoration_style,
                    ),
                ) else {
                    return Ok(None);
                };
                components.push(style);
            }
            match computed.text_decoration_thickness {
                raikiri_style::ComputedTextDecorationThickness::Auto => {}
                raikiri_style::ComputedTextDecorationThickness::FromFont => {
                    components.push("from-font".to_owned());
                }
                raikiri_style::ComputedTextDecorationThickness::Length(length) => {
                    components.push(format!("{}px", length.px()));
                }
            }
            let color = match computed.text_decoration_color {
                raikiri_style::property::TextDecorationColor::CurrentColor => None,
                raikiri_style::property::TextDecorationColor::Resolved(color) => Some(color),
                _ => return Ok(None),
            };
            if let Some(color) = color {
                if color.a == 255 {
                    components.push(format!("rgb({}, {}, {})", color.r, color.g, color.b));
                } else {
                    let alpha = f64::from(color.a) / 255.0;
                    components.push(format!(
                        "rgba({}, {}, {}, {alpha})",
                        color.r, color.g, color.b
                    ));
                }
            }
            if components.is_empty() {
                components.push("none".to_owned());
            }
            return Ok(Some(components.join(" ")));
        }
        if property.eq_ignore_ascii_case("text-decoration-style") {
            let Some(value) = raikiri_style::property::serialize_value(
                &raikiri_style::property::PropertyValue::TextDecorationStyle(
                    computed.text_decoration_style,
                ),
            ) else {
                return Ok(None);
            };
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("text-decoration-line") {
            let Some(value) = raikiri_style::property::serialize_value(
                &raikiri_style::property::PropertyValue::TextDecorationLine(
                    computed.text_decoration_line,
                ),
            ) else {
                return Ok(None);
            };
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("text-decoration-inset") {
            let value = match computed.text_decoration_inset {
                raikiri_style::ComputedTextDecorationInset::Auto => "auto".to_owned(),
                raikiri_style::ComputedTextDecorationInset::Lengths { start, end } => {
                    let mut font_context = self.setup.font_context.clone();
                    let start_px = computed.text_decoration_inset_start_ch.as_ref().map_or(
                        start.px(),
                        |provenance| {
                            provenance.factor
                                * raikiri_dom::layout::measure_ch_advance_for_font_key(
                                    &mut font_context,
                                    &provenance.font,
                                )
                        },
                    );
                    let end_px = computed.text_decoration_inset_end_ch.as_ref().map_or(
                        end.px(),
                        |provenance| {
                            provenance.factor
                                * raikiri_dom::layout::measure_ch_advance_for_font_key(
                                    &mut font_context,
                                    &provenance.font,
                                )
                        },
                    );
                    if start_px == end_px {
                        format!("{start_px}px")
                    } else {
                        format!("{start_px}px {end_px}px")
                    }
                }
            };
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("text-emphasis-position") {
            let Some(value) = raikiri_style::property::serialize_value(
                &raikiri_style::property::PropertyValue::TextEmphasisPosition(
                    computed.text_emphasis_position,
                ),
            ) else {
                return Ok(None);
            };
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("text-shadow") {
            if computed.text_shadow.is_empty() {
                return Ok(Some("none".to_owned()));
            }
            let mut shadows = Vec::with_capacity(computed.text_shadow.len());
            for shadow in computed.text_shadow.iter() {
                let color = match shadow.color {
                    raikiri_style::property::TextShadowColor::CurrentColor => computed.color,
                    raikiri_style::property::TextShadowColor::Resolved(color) => color,
                    _ => return Ok(None),
                };
                let color = if color.a == 255 {
                    format!("rgb({}, {}, {})", color.r, color.g, color.b)
                } else {
                    let alpha = f64::from(color.a) / 255.0;
                    format!("rgba({}, {}, {}, {alpha})", color.r, color.g, color.b)
                };
                shadows.push(format!(
                    "{color} {}px {}px {}px",
                    shadow.offset_x.px(),
                    shadow.offset_y.px(),
                    shadow.blur_radius.px()
                ));
            }
            return Ok(Some(shadows.join(", ")));
        }
        if property.eq_ignore_ascii_case("text-emphasis-style") {
            let Some(value) = raikiri_style::property::serialize_value(
                &raikiri_style::property::PropertyValue::TextEmphasisStyle(
                    computed.text_emphasis_style.clone(),
                ),
            ) else {
                return Ok(None);
            };
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("text-emphasis") {
            let Some(style) = raikiri_style::property::serialize_value(
                &raikiri_style::property::PropertyValue::TextEmphasisStyle(
                    computed.text_emphasis_style.clone(),
                ),
            ) else {
                return Ok(None);
            };
            let color = match computed.text_emphasis_color {
                raikiri_style::property::TextDecorationColor::CurrentColor => computed.color,
                raikiri_style::property::TextDecorationColor::Resolved(color) => color,
                _ => return Ok(None),
            };
            let color = if color.a == 255 {
                format!("rgb({}, {}, {})", color.r, color.g, color.b)
            } else {
                let alpha = f64::from(color.a) / 255.0;
                format!("rgba({}, {}, {}, {alpha})", color.r, color.g, color.b)
            };
            return Ok(Some(format!("{style} {color}")));
        }
        if property.eq_ignore_ascii_case("text-underline-position") {
            let Some(value) = raikiri_style::property::serialize_value(
                &raikiri_style::property::PropertyValue::TextUnderlinePosition(
                    computed.text_underline_position,
                ),
            ) else {
                return Ok(None);
            };
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("text-underline-offset") {
            let value = match computed.text_underline_offset {
                raikiri_style::ComputedTextUnderlineOffset::Auto => {
                    raikiri_style::property::TextUnderlineOffset::Auto
                }
                raikiri_style::ComputedTextUnderlineOffset::Length(length) => {
                    raikiri_style::property::TextUnderlineOffset::Length(
                        raikiri_style::property::Length::Px(length.px()),
                    )
                }
                raikiri_style::ComputedTextUnderlineOffset::Percent(percent) => {
                    raikiri_style::property::TextUnderlineOffset::Length(
                        raikiri_style::property::Length::Percent(percent),
                    )
                }
                raikiri_style::ComputedTextUnderlineOffset::Calc(calc) => {
                    raikiri_style::property::TextUnderlineOffset::Calc(
                        raikiri_style::property::LengthPercentageCalc {
                            percent: calc.percent,
                            px: calc.px,
                            em: 0.0,
                        },
                    )
                }
            };
            let Some(value) = raikiri_style::property::serialize_value(
                &raikiri_style::property::PropertyValue::TextUnderlineOffset(value),
            ) else {
                return Ok(None);
            };
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("text-decoration-color") {
            let color = match computed.text_decoration_color {
                raikiri_style::property::TextDecorationColor::CurrentColor => computed.color,
                raikiri_style::property::TextDecorationColor::Resolved(color) => color,
                _ => return Ok(None),
            };
            let value = if color.a == 255 {
                format!("rgb({}, {}, {})", color.r, color.g, color.b)
            } else {
                let alpha = f64::from(color.a) / 255.0;
                format!("rgba({}, {}, {}, {alpha})", color.r, color.g, color.b)
            };
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("letter-spacing") {
            let value = match computed.letter_spacing_computed {
                raikiri_style::ComputedLetterSpacing::Px(0.0) => "normal".to_owned(),
                raikiri_style::ComputedLetterSpacing::Px(px) => format!("{px}px"),
                raikiri_style::ComputedLetterSpacing::Percent(percent) => format!("{percent}%"),
                raikiri_style::ComputedLetterSpacing::Calc(calc) => {
                    let operator = if calc.px.is_sign_negative() {
                        " - "
                    } else {
                        " + "
                    };
                    format!("calc({}%{}{}px)", calc.percent, operator, calc.px.abs())
                }
            };
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("word-spacing") {
            let value = match computed.word_spacing_computed {
                raikiri_style::ComputedWordSpacing::Px(px) => format!("{px}px"),
                raikiri_style::ComputedWordSpacing::Percent(percent) => format!("{percent}%"),
                raikiri_style::ComputedWordSpacing::Calc(calc) => {
                    let operator = if calc.px.is_sign_negative() {
                        " - "
                    } else {
                        " + "
                    };
                    format!("calc({}%{}{}px)", calc.percent, operator, calc.px.abs())
                }
            };
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("text-indent") {
            let mut value = match computed.text_indent {
                raikiri_style::ComputedTextIndent::Px(px) => format!("{px}px"),
                raikiri_style::ComputedTextIndent::Percent(percent) => {
                    format!("{percent}%")
                }
                raikiri_style::ComputedTextIndent::Calc(calc) => {
                    let operator = if calc.px.is_sign_negative() {
                        " - "
                    } else {
                        " + "
                    };
                    format!("calc({}%{}{}px)", calc.percent, operator, calc.px.abs())
                }
            };
            if computed.text_indent_hanging {
                value.push_str(" hanging");
            }
            if computed.text_indent_each_line {
                value.push_str(" each-line");
            }
            return Ok(Some(value));
        }
        if property.eq_ignore_ascii_case("tab-size") {
            let value = match computed.tab_size {
                raikiri_style::ComputedTabSize::Number(number) => format!("{number}"),
                raikiri_style::ComputedTabSize::Length(length) => format!("{}px", length.0),
            };
            return Ok(Some(value));
        }
        let value = if property.eq_ignore_ascii_case("white-space") {
            match computed.white_space {
                raikiri_style::property::WhiteSpace::Normal => "normal",
                raikiri_style::property::WhiteSpace::Pre => "pre",
                raikiri_style::property::WhiteSpace::Nowrap => "nowrap",
                raikiri_style::property::WhiteSpace::PreWrap => "pre-wrap",
                raikiri_style::property::WhiteSpace::PreLine => "pre-line",
                raikiri_style::property::WhiteSpace::BreakSpaces => "break-spaces",
                _ => return Ok(None),
            }
        } else if property.eq_ignore_ascii_case("white-space-collapse") {
            match computed.white_space_collapse {
                raikiri_style::property::WhiteSpaceCollapse::Collapse => "collapse",
                raikiri_style::property::WhiteSpaceCollapse::Discard => "discard",
                raikiri_style::property::WhiteSpaceCollapse::Preserve => "preserve",
                raikiri_style::property::WhiteSpaceCollapse::PreserveBreaks => "preserve-breaks",
                raikiri_style::property::WhiteSpaceCollapse::PreserveSpaces => "preserve-spaces",
                raikiri_style::property::WhiteSpaceCollapse::BreakSpaces => "break-spaces",
                _ => return Ok(None),
            }
        } else if property.eq_ignore_ascii_case("line-break") {
            match computed.line_break {
                raikiri_style::property::LineBreak::Auto => "auto",
                raikiri_style::property::LineBreak::Loose => "loose",
                raikiri_style::property::LineBreak::Normal => "normal",
                raikiri_style::property::LineBreak::Strict => "strict",
                raikiri_style::property::LineBreak::Anywhere => "anywhere",
                _ => return Ok(None),
            }
        } else if property.eq_ignore_ascii_case("hyphens") {
            match computed.hyphens {
                raikiri_style::property::Hyphens::None => "none",
                raikiri_style::property::Hyphens::Manual => "manual",
                raikiri_style::property::Hyphens::Auto => "auto",
                _ => return Ok(None),
            }
        } else if property.eq_ignore_ascii_case("overflow-wrap")
            || property.eq_ignore_ascii_case("word-wrap")
        {
            match computed.overflow_wrap {
                raikiri_style::property::OverflowWrap::Normal => "normal",
                raikiri_style::property::OverflowWrap::BreakWord => "break-word",
                raikiri_style::property::OverflowWrap::Anywhere => "anywhere",
                _ => return Ok(None),
            }
        } else if property.eq_ignore_ascii_case("word-break") {
            match computed.word_break {
                raikiri_style::property::WordBreak::Normal => "normal",
                raikiri_style::property::WordBreak::KeepAll => "keep-all",
                raikiri_style::property::WordBreak::BreakAll => "break-all",
                raikiri_style::property::WordBreak::Manual => "manual",
                raikiri_style::property::WordBreak::AutoPhrase => "auto-phrase",
                raikiri_style::property::WordBreak::BreakWord => "break-word",
                _ => return Ok(None),
            }
        } else if property.eq_ignore_ascii_case("text-align") {
            match computed.text_align {
                raikiri_style::property::TextAlign::Start => "start",
                raikiri_style::property::TextAlign::End => "end",
                raikiri_style::property::TextAlign::Left => "left",
                raikiri_style::property::TextAlign::Right => "right",
                raikiri_style::property::TextAlign::Center => "center",
                raikiri_style::property::TextAlign::Justify => "justify",
                raikiri_style::property::TextAlign::JustifyAll => "justify-all",
                // Intermediate-only values must not leak as computed CSSOM values.
                _ => return Ok(None),
            }
        } else if property.eq_ignore_ascii_case("text-wrap-mode") {
            match computed.text_wrap {
                raikiri_style::property::TextWrapMode::Wrap => "wrap",
                raikiri_style::property::TextWrapMode::Nowrap => "nowrap",
                _ => return Ok(None),
            }
        } else if property.eq_ignore_ascii_case("text-wrap-style") {
            match computed.text_wrap_style {
                raikiri_style::property::TextWrapStyle::Auto => "auto",
                raikiri_style::property::TextWrapStyle::Balance => "balance",
                raikiri_style::property::TextWrapStyle::Pretty => "pretty",
                raikiri_style::property::TextWrapStyle::Stable => "stable",
                _ => return Ok(None),
            }
        } else if property.eq_ignore_ascii_case("text-align-last") {
            match computed.text_align_last {
                raikiri_style::property::TextAlignLast::Auto => "auto",
                raikiri_style::property::TextAlignLast::Start => "start",
                raikiri_style::property::TextAlignLast::End => "end",
                raikiri_style::property::TextAlignLast::Left => "left",
                raikiri_style::property::TextAlignLast::Right => "right",
                raikiri_style::property::TextAlignLast::Center => "center",
                raikiri_style::property::TextAlignLast::Justify => "justify",
                raikiri_style::property::TextAlignLast::MatchParent => "match-parent",
                _ => return Ok(None),
            }
        } else if property.eq_ignore_ascii_case("text-justify") {
            match computed.text_justify {
                raikiri_style::property::TextJustify::Auto => "auto",
                raikiri_style::property::TextJustify::None => "none",
                raikiri_style::property::TextJustify::InterWord => "inter-word",
                raikiri_style::property::TextJustify::InterCharacter => "inter-character",
                // CSS Text 3 defines legacy `distribute` as computing to `inter-character`.
                raikiri_style::property::TextJustify::Distribute => "inter-character",
                _ => return Ok(None),
            }
        } else {
            match computed.text_transform {
                raikiri_style::property::TextTransform::None => "none",
                raikiri_style::property::TextTransform::MathAuto => "math-auto",
                raikiri_style::property::TextTransform::Capitalize => "capitalize",
                raikiri_style::property::TextTransform::Uppercase => "uppercase",
                raikiri_style::property::TextTransform::Lowercase => "lowercase",
                raikiri_style::property::TextTransform::FullWidth => "full-width",
                raikiri_style::property::TextTransform::FullSizeKana => "full-size-kana",
                raikiri_style::property::TextTransform::CapitalizeFullWidth => {
                    "capitalize full-width"
                }
                raikiri_style::property::TextTransform::UppercaseFullWidth => {
                    "uppercase full-width"
                }
                raikiri_style::property::TextTransform::LowercaseFullWidth => {
                    "lowercase full-width"
                }
                raikiri_style::property::TextTransform::CapitalizeFullSizeKana => {
                    "capitalize full-size-kana"
                }
                raikiri_style::property::TextTransform::UppercaseFullSizeKana => {
                    "uppercase full-size-kana"
                }
                raikiri_style::property::TextTransform::LowercaseFullSizeKana => {
                    "lowercase full-size-kana"
                }
                raikiri_style::property::TextTransform::FullWidthFullSizeKana => {
                    "full-width full-size-kana"
                }
                raikiri_style::property::TextTransform::CapitalizeFullWidthFullSizeKana => {
                    "capitalize full-width full-size-kana"
                }
                raikiri_style::property::TextTransform::UppercaseFullWidthFullSizeKana => {
                    "uppercase full-width full-size-kana"
                }
                raikiri_style::property::TextTransform::LowercaseFullWidthFullSizeKana => {
                    "lowercase full-width full-size-kana"
                }
                _ => return Ok(None),
            }
        };
        Ok(Some(value.to_owned()))
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
    run_testharness_file_with_helper(path, wpt_root, "")
}

fn run_testharness_file_with_helper(
    path: &Path,
    wpt_root: &Path,
    helper_script: &str,
) -> TestHarnessFileResult {
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
    let inline_script = inline_test_scripts(&html);
    let script = if helper_script.is_empty() {
        inline_script
    } else {
        format!("{helper_script}\n{inline_script}")
    };
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
mod tests;
