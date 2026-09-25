//! Static SVG parsing and bounded rasterization.
//!
//! Backend types stay private to this crate. SVG references that could read
//! local files, issue requests, or recursively load data URLs are rejected.

use raikiri_traits::DecodedImage;
use std::ops::Range;

const DEFAULT_MAX_OUTPUT_BYTES: u64 = 32 * 1024 * 1024;
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";

/// Natural dimensions and ratio declared by an SVG source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SvgIntrinsicSize {
    /// Absolute natural width in CSS pixels, when present.
    pub width: Option<f32>,
    /// Absolute natural height in CSS pixels, when present.
    pub height: Option<f32>,
    /// Intrinsic width-to-height ratio, when present.
    pub aspect_ratio: Option<f32>,
}

/// The concrete viewport used to rasterize an SVG.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SvgViewport {
    /// CSS pixel width.
    pub width: f32,
    /// CSS pixel height.
    pub height: f32,
}

/// Host styles passed into an SVG image or inline SVG root.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SvgRootStyle {
    /// The inherited CSS `color`, in straight RGBA8.
    pub inherited_color: [u8; 4],
    /// Host computed opacity, applied once to the completed raster.
    pub opacity: f32,
    /// Neutralize the source root opacity when the caller composites the
    /// computed opacity around the SVG and its box decorations as one group.
    pub neutralize_root_opacity: bool,
    /// Whether the host computed visibility permits painting.
    pub visible: bool,
}

impl Default for SvgRootStyle {
    fn default() -> Self {
        Self {
            inherited_color: [0, 0, 0, 255],
            opacity: 1.0,
            neutralize_root_opacity: false,
            visible: true,
        }
    }
}

/// SVG parsing, viewport, or allocation failure.
#[derive(Debug)]
#[non_exhaustive]
pub enum SvgError {
    /// The source is not a supported SVG document.
    InvalidDocument(String),
    /// The document contains a DTD, which is outside the supported subset.
    UnsupportedDoctype,
    /// An SVG `<image>` reference would access a resource outside this file.
    ExternalReference,
    /// Filter effects are outside the supported static SVG subset.
    UnsupportedFilterEffects,
    /// The requested viewport is zero, negative, non-finite, or out of range.
    InvalidViewport,
    /// The host opacity is not a finite CSS opacity value.
    InvalidOpacity,
    /// The checked raster size exceeds the configured output limit.
    OutputLimitExceeded {
        /// Requested output bytes.
        bytes: u64,
        /// Maximum permitted output bytes.
        limit: u64,
    },
    /// The bounded output buffer could not be allocated.
    AllocationFailed,
}

impl std::fmt::Display for SvgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDocument(message) => write!(f, "invalid SVG document: {message}"),
            Self::UnsupportedDoctype => f.write_str("SVG documents with a DOCTYPE are unsupported"),
            Self::ExternalReference => {
                f.write_str("SVG image references outside the document are disabled")
            }
            Self::UnsupportedFilterEffects => {
                f.write_str("SVG filter effects are outside the supported subset")
            }
            Self::InvalidViewport => f.write_str("invalid SVG raster viewport"),
            Self::InvalidOpacity => f.write_str("invalid SVG host opacity"),
            Self::OutputLimitExceeded { bytes, limit } => {
                write!(
                    f,
                    "SVG output is {bytes} bytes, exceeding the {limit}-byte limit"
                )
            }
            Self::AllocationFailed => f.write_str("could not allocate SVG raster output"),
        }
    }
}

impl std::error::Error for SvgError {}

/// Parsed SVG source whose raster backend remains private to this crate.
#[derive(Debug)]
pub struct SvgDocument {
    tree: usvg::Tree,
    source: String,
    intrinsic: SvgIntrinsicSize,
    has_view_box: bool,
}

impl SvgDocument {
    /// Parses and validates a UTF-8 SVG document.
    pub fn parse(data: &[u8]) -> Result<Self, SvgError> {
        let source = std::str::from_utf8(data)
            .map_err(|error| SvgError::InvalidDocument(error.to_string()))?;
        if contains_doctype_declaration(source) {
            return Err(SvgError::UnsupportedDoctype);
        }

        let xml = roxmltree::Document::parse(source)
            .map_err(|error| SvgError::InvalidDocument(error.to_string()))?;
        let root = xml.root_element();
        if root.tag_name().name() != "svg"
            || root
                .tag_name()
                .namespace()
                .is_some_and(|ns| ns != SVG_NAMESPACE)
        {
            return Err(SvgError::InvalidDocument(
                "document root is not an SVG element".to_owned(),
            ));
        }
        reject_external_image_references(root)?;
        reject_filter_effects(root)?;

        let intrinsic = intrinsic_size(root);
        let has_view_box = root
            .attribute("viewBox")
            .and_then(parse_view_box_ratio)
            .is_some();
        let tree = parse_tree(source)?;
        if !tree.filters().is_empty() {
            return Err(SvgError::UnsupportedFilterEffects);
        }
        let tree_size = tree.size();
        if !tree_size.width().is_finite()
            || !tree_size.height().is_finite()
            || tree_size.width() <= 0.0
            || tree_size.height() <= 0.0
        {
            return Err(SvgError::InvalidDocument(
                "SVG has no positive renderable size".to_owned(),
            ));
        }

        Ok(Self {
            tree,
            source: source.to_owned(),
            intrinsic,
            has_view_box,
        })
    }

    /// Returns the source's natural dimensions and ratio without CSS defaults.
    pub fn intrinsic_size(&self) -> SvgIntrinsicSize {
        self.intrinsic
    }

    /// Rasterizes the SVG to straight-alpha RGBA8 at `viewport`.
    ///
    /// The effective output limit is the smaller of 32 MiB and
    /// `max_output_bytes`. Pixel dimensions are rounded up before checking
    /// multiplication overflow and allocating a pixmap.
    pub fn rasterize(
        &self,
        viewport: SvgViewport,
        root_style: SvgRootStyle,
        max_output_bytes: Option<u64>,
    ) -> Result<DecodedImage, SvgError> {
        if !root_style.opacity.is_finite() || !(0.0..=1.0).contains(&root_style.opacity) {
            return Err(SvgError::InvalidOpacity);
        }

        let (width, height, byte_len) = checked_output_size(viewport, max_output_bytes)?;
        let mut pixels = allocate_transparent_pixels(byte_len)?;

        if root_style.visible && root_style.opacity > 0.0 {
            let source = if viewport_matches_tree(&self.tree, width, height, self.has_view_box) {
                self.source.clone()
            } else {
                with_root_viewport_size(&self.source, width, height)?
            };
            let source = if root_style.neutralize_root_opacity {
                with_root_opacity_neutralized(&source)?
            } else {
                source
            };
            let source = with_inherited_color(&source, root_style.inherited_color)?;
            if source == self.source {
                render_tree(&self.tree, width, height, root_style.opacity, &mut pixels)?;
            } else {
                let tree = parse_tree(&source)?;
                render_tree(&tree, width, height, root_style.opacity, &mut pixels)?;
            }
        }

        Ok(DecodedImage {
            width,
            height,
            rgba: pixels,
        })
    }
}

fn parse_tree(source: &str) -> Result<usvg::Tree, SvgError> {
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    usvg::Tree::from_str(source, &options)
        .map_err(|error| SvgError::InvalidDocument(error.to_string()))
}

fn contains_doctype_declaration(source: &str) -> bool {
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find("<") {
        let start = cursor + relative;
        let remaining = &source[start..];
        if let Some(comment) = remaining.strip_prefix("<!--") {
            cursor = start + 4 + comment.find("-->").map_or(comment.len(), |end| end + 3);
            continue;
        }
        if let Some(cdata) = remaining.strip_prefix("<![CDATA[") {
            cursor = start + 9 + cdata.find("]]>").map_or(cdata.len(), |end| end + 3);
            continue;
        }
        if let Some(processing_instruction) = remaining.strip_prefix("<?") {
            cursor = start
                + 2
                + processing_instruction
                    .find("?>")
                    .map_or(processing_instruction.len(), |end| end + 2);
            continue;
        }
        if remaining.starts_with("<!DOCTYPE")
            && remaining
                .as_bytes()
                .get("<!DOCTYPE".len())
                .is_some_and(u8::is_ascii_whitespace)
        {
            return true;
        }
        cursor = start + 1;
    }
    false
}

fn reject_external_image_references(root: roxmltree::Node<'_, '_>) -> Result<(), SvgError> {
    for node in root.descendants().filter(|node| node.is_element()) {
        if node.tag_name().name() != "image"
            || node
                .tag_name()
                .namespace()
                .is_some_and(|namespace| namespace != SVG_NAMESPACE)
        {
            continue;
        }

        let href = node
            .attribute("href")
            .or_else(|| node.attribute((XLINK_NAMESPACE, "href")));
        if href.is_some_and(|href| !href.trim().starts_with('#')) {
            return Err(SvgError::ExternalReference);
        }
    }
    Ok(())
}

fn reject_filter_effects(root: roxmltree::Node<'_, '_>) -> Result<(), SvgError> {
    for node in root.descendants().filter(|node| node.is_element()) {
        let is_svg = node
            .tag_name()
            .namespace()
            .is_none_or(|namespace| namespace == SVG_NAMESPACE);
        let is_style = node.tag_name().name() == "style"
            && node
                .attribute("type")
                .is_none_or(|style_type| style_type == "text/css");
        if !is_svg && !is_style {
            continue;
        }

        if (is_svg
            && (node
                .attribute("filter")
                .is_some_and(|value| !value.trim().eq_ignore_ascii_case("none"))
                || node
                    .attribute("style")
                    .is_some_and(css_declarations_use_filter_effects)))
            || (is_style && node.text().is_some_and(css_stylesheet_uses_filter_effects))
        {
            return Err(SvgError::UnsupportedFilterEffects);
        }
    }
    Ok(())
}

const MAX_FILTER_CSS_NESTING: usize = 64;

struct FilterCssParser {
    nesting: usize,
}

impl<'i> cssparser::DeclarationParser<'i> for FilterCssParser {
    type Declaration = bool;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: cssparser::CowRcStr<'i>,
        input: &mut cssparser::Parser<'i, 't>,
        _declaration_start: &cssparser::ParserState,
    ) -> Result<bool, cssparser::ParseError<'i, Self::Error>> {
        if !name.eq_ignore_ascii_case("filter") {
            while input.next_including_whitespace_and_comments().is_ok() {}
            return Ok(false);
        }

        let is_none = input
            .parse_until_before(cssparser::Delimiter::Bang, |value| {
                let keyword = value.expect_ident_cloned()?;
                value.expect_exhausted()?;
                Ok::<_, cssparser::ParseError<'i, Self::Error>>(
                    keyword.eq_ignore_ascii_case("none"),
                )
            })
            .unwrap_or(false);
        input.try_parse(cssparser::parse_important).ok();
        input.expect_exhausted()?;
        Ok(!is_none)
    }
}

impl<'i> cssparser::AtRuleParser<'i> for FilterCssParser {
    type Prelude = ();
    type AtRule = bool;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        _name: cssparser::CowRcStr<'i>,
        input: &mut cssparser::Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, Self::Error>> {
        while input.next().is_ok() {}
        Ok(())
    }

    fn parse_block<'t>(
        &mut self,
        _prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut cssparser::Parser<'i, 't>,
    ) -> Result<Self::AtRule, cssparser::ParseError<'i, Self::Error>> {
        Ok(css_stylesheet_uses_filter_effects_at_depth(
            input,
            self.nesting + 1,
        ))
    }
}

impl<'i> cssparser::QualifiedRuleParser<'i> for FilterCssParser {
    type Prelude = ();
    type QualifiedRule = bool;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut cssparser::Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, Self::Error>> {
        while input.next().is_ok() {}
        Ok(())
    }

    fn parse_block<'t>(
        &mut self,
        _prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut cssparser::Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, cssparser::ParseError<'i, Self::Error>> {
        Ok(css_declarations_use_filter_effects_at_depth(
            input,
            self.nesting + 1,
        ))
    }
}

impl<'i> cssparser::RuleBodyItemParser<'i, bool, ()> for FilterCssParser {
    fn parse_qualified(&self) -> bool {
        true
    }

    fn parse_declarations(&self) -> bool {
        true
    }
}

fn css_declarations_use_filter_effects(source: &str) -> bool {
    let mut input = cssparser::ParserInput::new(source);
    let mut parser = cssparser::Parser::new(&mut input);
    css_declarations_use_filter_effects_at_depth(&mut parser, 0)
}

fn css_declarations_use_filter_effects_at_depth(
    input: &mut cssparser::Parser<'_, '_>,
    nesting: usize,
) -> bool {
    if nesting >= MAX_FILTER_CSS_NESTING {
        return true;
    }
    let mut declaration_parser = FilterCssParser { nesting };
    cssparser::RuleBodyParser::new(input, &mut declaration_parser)
        .filter_map(Result::ok)
        .any(|uses_filter| uses_filter)
}

fn css_stylesheet_uses_filter_effects(source: &str) -> bool {
    let mut input = cssparser::ParserInput::new(source);
    let mut parser = cssparser::Parser::new(&mut input);
    css_stylesheet_uses_filter_effects_at_depth(&mut parser, 0)
}

fn css_stylesheet_uses_filter_effects_at_depth(
    input: &mut cssparser::Parser<'_, '_>,
    nesting: usize,
) -> bool {
    if nesting >= MAX_FILTER_CSS_NESTING {
        return true;
    }
    let mut stylesheet_parser = FilterCssParser { nesting };
    cssparser::StyleSheetParser::new(input, &mut stylesheet_parser)
        .filter_map(Result::ok)
        .any(|uses_filter| uses_filter)
}

fn intrinsic_size(root: roxmltree::Node<'_, '_>) -> SvgIntrinsicSize {
    let width = root.attribute("width").and_then(parse_absolute_length);
    let height = root.attribute("height").and_then(parse_absolute_length);
    let view_box_ratio = root.attribute("viewBox").and_then(parse_view_box_ratio);
    let aspect_ratio = match (width, height) {
        (Some(width), Some(height)) => positive_ratio(width, height),
        _ => view_box_ratio,
    };

    SvgIntrinsicSize {
        width,
        height,
        aspect_ratio,
    }
}

fn parse_absolute_length(value: &str) -> Option<f32> {
    let value = value.trim();
    let units = [
        ("px", 1.0_f64),
        ("in", 96.0),
        ("cm", 96.0 / 2.54),
        ("mm", 96.0 / 25.4),
        ("q", 96.0 / 101.6),
        ("pt", 96.0 / 72.0),
        ("pc", 16.0),
    ];
    let lower = value.to_ascii_lowercase();
    let (number, factor) = units
        .iter()
        .find_map(|(unit, factor)| lower.strip_suffix(unit).map(|number| (number, *factor)))
        .unwrap_or((value, 1.0));
    let length = number.trim().parse::<f64>().ok()? * factor;
    if !length.is_finite() || length <= 0.0 || length > f32::MAX as f64 {
        return None;
    }
    Some(length as f32)
}

fn parse_view_box_ratio(value: &str) -> Option<f32> {
    let mut values = value
        .split(|character: char| character.is_ascii_whitespace() || character == ',')
        .filter(|part| !part.is_empty())
        .map(str::parse::<f32>);
    let x = values.next()?.ok()?;
    let y = values.next()?.ok()?;
    let width = values.next()?.ok()?;
    let height = values.next()?.ok()?;
    if values.next().is_some() || !x.is_finite() || !y.is_finite() {
        return None;
    }
    positive_ratio(width, height)
}

fn positive_ratio(width: f32, height: f32) -> Option<f32> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let ratio = width / height;
    (ratio.is_finite() && ratio > 0.0).then_some(ratio)
}

fn viewport_matches_tree(tree: &usvg::Tree, width: u32, height: u32, has_view_box: bool) -> bool {
    let size = tree.size();
    let requested_width = f64::from(width);
    let requested_height = f64::from(height);
    let source_width = f64::from(size.width());
    let source_height = f64::from(size.height());
    if !has_view_box {
        return (requested_width - source_width).abs() <= source_width.max(1.0) * 1.0e-6
            && (requested_height - source_height).abs() <= source_height.max(1.0) * 1.0e-6;
    }
    let first = requested_width * source_height;
    let second = requested_height * source_width;
    (first - second).abs() <= first.max(second).max(1.0) * 1.0e-6
}

fn with_root_viewport_size(source: &str, width: u32, height: u32) -> Result<String, SvgError> {
    let xml = roxmltree::Document::parse(source)
        .map_err(|error| SvgError::InvalidDocument(error.to_string()))?;
    let root = xml.root_element();
    let width_value = format!("{width}px");
    let height_value = format!("{height}px");
    let declarations = format!("width:{width_value}!important;height:{height_value}!important");
    let style_value = root.attribute("style").map_or_else(
        || declarations.clone(),
        |style| {
            let retained = strip_inline_style_properties(style, &["width", "height"]);
            append_inline_declarations(&retained, &declarations)
        },
    );
    let mut edits = Vec::<(Range<usize>, String)>::new();
    let mut inserted_attributes = String::new();

    for (name, value) in [("width", width_value), ("height", height_value)] {
        if let Some(attribute) = root.attributes().find(|attribute| attribute.name() == name) {
            edits.push((attribute.range(), format!("{name}=\"{value}\"")));
        } else {
            inserted_attributes.push_str(&format!(" {name}=\"{value}\""));
        }
    }

    if let Some(attribute) = root
        .attributes()
        .find(|attribute| attribute.name() == "style")
    {
        edits.push((
            attribute.range(),
            format!("style=\"{}\"", escape_xml_attribute(&style_value)),
        ));
    } else {
        inserted_attributes.push_str(&format!(
            " style=\"{}\"",
            escape_xml_attribute(&style_value)
        ));
    }

    if !inserted_attributes.is_empty() {
        let start = root.range().start;
        let end = root_start_tag_end(source, start)
            .ok_or_else(|| SvgError::InvalidDocument("unterminated SVG root tag".to_owned()))?;
        let insertion = if end > start && source.as_bytes()[end - 1] == b'/' {
            end - 1
        } else {
            end
        };
        edits.push((insertion..insertion, inserted_attributes));
    }

    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut result = source.to_owned();
    for (range, replacement) in edits {
        result.replace_range(range, &replacement);
    }
    Ok(result)
}

fn with_root_opacity_neutralized(source: &str) -> Result<String, SvgError> {
    let xml = roxmltree::Document::parse(source)
        .map_err(|error| SvgError::InvalidDocument(error.to_string()))?;
    let root = xml.root_element();
    let style_value = root.attribute("style").map_or_else(
        || "opacity:1!important".to_owned(),
        |style| {
            let retained = strip_inline_style_properties(style, &["opacity"]);
            append_inline_declarations(&retained, "opacity:1!important")
        },
    );
    let replacement = format!("style=\"{}\"", escape_xml_attribute(&style_value));
    let mut edits = Vec::<(Range<usize>, String)>::new();
    let mut inserted_root_attributes = String::new();
    if let Some(attribute) = root
        .attributes()
        .find(|attribute| attribute.name() == "style")
    {
        edits.push((attribute.range(), replacement));
    } else {
        inserted_root_attributes.push(' ');
        inserted_root_attributes.push_str(&replacement);
    }

    // usvg keeps the first `!important` declaration it encounters, even when
    // a later root inline declaration should win. Move every stylesheet
    // opacity declaration behind a unique descendant-only attribute instead:
    // the host opacity can then own the root while the same rules still apply
    // to matching SVG descendants. The selector specificity shift is uniform
    // for opacity rules, so their relative cascade order stays the same.
    let scope_attribute = unique_scope_attribute(source);
    let mut has_scoped_stylesheet_opacity = false;
    for node in root
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "style")
    {
        if node
            .attribute("type")
            .is_some_and(|value| value != "text/css")
        {
            continue;
        }
        let Some(stylesheet_text) = node.text() else {
            continue;
        };
        let mut stylesheet = simplecss::StyleSheet::parse(stylesheet_text);
        let mut descendant_opacity_rules = Vec::new();
        for rule in &mut stylesheet.rules {
            let opacity_declarations = rule
                .declarations
                .iter()
                .copied()
                .filter(|declaration| declaration.name == "opacity")
                .collect::<Vec<_>>();
            if opacity_declarations.is_empty() {
                continue;
            }

            rule.declarations
                .retain(|declaration| declaration.name != "opacity");
            let selector = format!("{}[{scope_attribute}]", rule.selector);
            let mut scoped_rule = format!("{selector} {{");
            for declaration in opacity_declarations {
                scoped_rule.push_str(declaration.name);
                scoped_rule.push(':');
                scoped_rule.push_str(declaration.value);
                if declaration.important {
                    scoped_rule.push_str(" !important");
                }
                scoped_rule.push(';');
            }
            scoped_rule.push('}');
            descendant_opacity_rules.push(scoped_rule);
        }

        if descendant_opacity_rules.is_empty() {
            continue;
        }
        has_scoped_stylesheet_opacity = true;
        let mut rewritten = stylesheet.to_string();
        for rule in descendant_opacity_rules {
            rewritten.push('\n');
            rewritten.push_str(&rule);
        }
        if let Some(text_node) = node.children().find(|child| child.is_text()) {
            edits.push((text_node.range(), escape_xml_text(&rewritten)));
        }
    }

    if has_scoped_stylesheet_opacity {
        for descendant in root.descendants().filter(|node| node.is_element()) {
            if descendant == root {
                continue;
            }
            let start = descendant.range().start;
            let end = root_start_tag_end(source, start).ok_or_else(|| {
                SvgError::InvalidDocument("unterminated SVG element tag".to_owned())
            })?;
            let insertion = if end > start && source.as_bytes()[end - 1] == b'/' {
                end - 1
            } else {
                end
            };
            edits.push((insertion..insertion, format!(" {scope_attribute}=\"\"")));
        }
    }

    if !inserted_root_attributes.is_empty() {
        let start = root.range().start;
        let end = root_start_tag_end(source, start)
            .ok_or_else(|| SvgError::InvalidDocument("unterminated SVG root tag".to_owned()))?;
        let insertion = if end > start && source.as_bytes()[end - 1] == b'/' {
            end - 1
        } else {
            end
        };
        edits.push((insertion..insertion, inserted_root_attributes));
    }

    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut result = source.to_owned();
    for (range, replacement) in edits {
        result.replace_range(range, &replacement);
    }
    Ok(result)
}

fn unique_scope_attribute(source: &str) -> String {
    let base = "data-raikiri-root-opacity-scope";
    if !source.contains(base) {
        return base.to_owned();
    }
    (1_u32..)
        .map(|suffix| format!("{base}-{suffix}"))
        .find(|candidate| !source.contains(candidate))
        .unwrap_or_else(|| format!("{base}-fallback"))
}

fn append_inline_declarations(style: &str, declarations: &str) -> String {
    if style.trim().is_empty() {
        declarations.to_owned()
    } else {
        format!("{style};{declarations}")
    }
}

fn strip_inline_style_properties(style: &str, properties: &[&str]) -> String {
    struct StripProperties<'a> {
        properties: &'a [&'a str],
    }

    impl<'i> cssparser::DeclarationParser<'i> for StripProperties<'_> {
        type Declaration = Option<String>;
        type Error = ();

        fn parse_value<'t>(
            &mut self,
            name: cssparser::CowRcStr<'i>,
            input: &mut cssparser::Parser<'i, 't>,
            declaration_start: &cssparser::ParserState,
        ) -> Result<Self::Declaration, cssparser::ParseError<'i, Self::Error>> {
            let remove = self
                .properties
                .iter()
                .any(|property| name.eq_ignore_ascii_case(property));
            while input.next_including_whitespace_and_comments().is_ok() {}
            if remove {
                return Ok(None);
            }

            let declaration = input
                .slice(declaration_start.position()..input.position())
                .trim()
                .to_owned();
            Ok((!declaration.is_empty()).then_some(declaration))
        }
    }

    impl<'i> cssparser::AtRuleParser<'i> for StripProperties<'_> {
        type Prelude = ();
        type AtRule = Option<String>;
        type Error = ();
    }

    impl<'i> cssparser::QualifiedRuleParser<'i> for StripProperties<'_> {
        type Prelude = ();
        type QualifiedRule = Option<String>;
        type Error = ();
    }

    impl<'i> cssparser::RuleBodyItemParser<'i, Option<String>, ()> for StripProperties<'_> {
        fn parse_qualified(&self) -> bool {
            false
        }

        fn parse_declarations(&self) -> bool {
            true
        }
    }

    let mut input = cssparser::ParserInput::new(style);
    let mut parser = cssparser::Parser::new(&mut input);
    let mut declaration_parser = StripProperties { properties };
    cssparser::RuleBodyParser::new(&mut parser, &mut declaration_parser)
        .filter_map(Result::ok)
        .flatten()
        .collect::<Vec<_>>()
        .join(";")
}

fn escape_xml_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

fn escape_xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn root_start_tag_end(source: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    source
        .as_bytes()
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, byte)| match (*byte, quote) {
            (b'\'', None) => {
                quote = Some(b'\'');
                None
            }
            (b'"', None) => {
                quote = Some(b'"');
                None
            }
            (b'\'', Some(b'\'')) | (b'"', Some(b'"')) => {
                quote = None;
                None
            }
            (b'>', None) => Some(index),
            _ => None,
        })
}

fn with_inherited_color(source: &str, color: [u8; 4]) -> Result<String, SvgError> {
    let xml = roxmltree::Document::parse(source)
        .map_err(|error| SvgError::InvalidDocument(error.to_string()))?;
    let root = xml.root_element();
    if root.attribute("color").is_some() {
        return Ok(source.to_owned());
    }

    let start = root.range().start;
    let bytes = source.as_bytes();
    let mut quote = None;
    let end = bytes
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, byte)| match (*byte, quote) {
            (b'\'', None) => {
                quote = Some(b'\'');
                None
            }
            (b'"', None) => {
                quote = Some(b'"');
                None
            }
            (b'\'', Some(b'\'')) | (b'"', Some(b'"')) => {
                quote = None;
                None
            }
            (b'>', None) => Some(index),
            _ => None,
        })
        .ok_or_else(|| SvgError::InvalidDocument("unterminated SVG root tag".to_owned()))?;
    let insertion = if end > start && bytes[end - 1] == b'/' {
        end - 1
    } else {
        end
    };

    let [red, green, blue, alpha] = color;
    let color_attribute = format!(
        " color=\"rgba({red}, {green}, {blue}, {:.6})\"",
        f32::from(alpha) / 255.0
    );
    let mut result = String::with_capacity(source.len() + color_attribute.len());
    result.push_str(&source[..insertion]);
    result.push_str(&color_attribute);
    result.push_str(&source[insertion..]);
    Ok(result)
}

fn checked_output_size(
    viewport: SvgViewport,
    caller_limit: Option<u64>,
) -> Result<(u32, u32, usize), SvgError> {
    if !viewport.width.is_finite()
        || !viewport.height.is_finite()
        || viewport.width <= 0.0
        || viewport.height <= 0.0
    {
        return Err(SvgError::InvalidViewport);
    }

    let rounded_width = viewport.width.ceil();
    let rounded_height = viewport.height.ceil();
    if rounded_width > i32::MAX as f32 / 4.0 || rounded_height > i32::MAX as f32 / 4.0 {
        return Err(SvgError::InvalidViewport);
    }
    let width = rounded_width as u32;
    let height = rounded_height as u32;
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(SvgError::InvalidViewport)?;
    let limit = caller_limit.map_or(DEFAULT_MAX_OUTPUT_BYTES, |value| {
        value.min(DEFAULT_MAX_OUTPUT_BYTES)
    });
    if bytes > limit {
        return Err(SvgError::OutputLimitExceeded { bytes, limit });
    }
    let byte_len = usize::try_from(bytes).map_err(|_| SvgError::InvalidViewport)?;
    Ok((width, height, byte_len))
}

fn allocate_transparent_pixels(byte_len: usize) -> Result<Vec<u8>, SvgError> {
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(byte_len)
        .map_err(|_| SvgError::AllocationFailed)?;
    pixels.resize(byte_len, 0);
    Ok(pixels)
}

fn render_tree(
    tree: &usvg::Tree,
    width: u32,
    height: u32,
    opacity: f32,
    pixels: &mut Vec<u8>,
) -> Result<(), SvgError> {
    let size =
        resvg::tiny_skia::IntSize::from_wh(width, height).ok_or(SvgError::InvalidViewport)?;
    let backing = std::mem::take(pixels);
    let mut pixmap =
        resvg::tiny_skia::Pixmap::from_vec(backing, size).ok_or(SvgError::AllocationFailed)?;
    let svg_size = tree.size();
    let transform = resvg::tiny_skia::Transform::from_scale(
        width as f32 / svg_size.width(),
        height as f32 / svg_size.height(),
    );
    resvg::render(tree, transform, &mut pixmap.as_mut());

    if opacity < 1.0 {
        for pixel in pixmap.data_mut().chunks_exact_mut(4) {
            for channel in pixel {
                *channel = (f32::from(*channel) * opacity).round() as u8;
            }
        }
    }
    *pixels = pixmap.take_demultiplied();
    Ok(())
}

#[cfg(test)]
mod tests;
