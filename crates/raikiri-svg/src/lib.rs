//! Static SVG parsing and bounded rasterization.
//!
//! Backend types stay private to this crate. SVG references that could read
//! local files, issue requests, or recursively load data URLs are rejected.

use raikiri_traits::DecodedImage;
use std::ops::Range;

const DEFAULT_MAX_OUTPUT_BYTES: u64 = 32 * 1024 * 1024;
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";
const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

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
    /// Host computed opacity. When root opacity is neutralized, this remains
    /// the SVG root's inherited value and the caller composites it externally.
    /// Otherwise it is applied once to the completed raster.
    pub opacity: f32,
    /// Neutralize the source root opacity when the caller composites the
    /// computed opacity around the SVG and its box decorations as one group.
    pub neutralize_root_opacity: bool,
    /// The host controls the SVG root's `background-color`, including when it
    /// computes to transparent, so omit the source background from the raster.
    pub host_controls_root_background: bool,
    /// Whether the host computed visibility permits painting.
    pub visible: bool,
}

impl Default for SvgRootStyle {
    fn default() -> Self {
        Self {
            inherited_color: [0, 0, 0, 255],
            opacity: 1.0,
            neutralize_root_opacity: false,
            host_controls_root_background: false,
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
    /// When `root_style.neutralize_root_opacity` is set, the output excludes
    /// the host opacity so the caller can composite it with the SVG's box.
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
            // Keep the original XML intact for selector matching whenever its
            // root opacity can be removed from the rendered pixels. Rewriting
            // opacity declarations into inline styles changes matches for
            // selectors such as `[style]` and `[opacity="0.5"]`.
            let normalize_zero_root_opacity =
                root_style.neutralize_root_opacity && self.tree.root().opacity().get() == 0.0;
            let source = if viewport_matches_tree(&self.tree, width, height, self.has_view_box) {
                self.source.clone()
            } else {
                with_root_viewport_size(&self.source, width, height)?
            };
            let source = if normalize_zero_root_opacity {
                normalize_svg_opacity_cascade(&source)?
            } else {
                source
            };
            let source = if normalize_zero_root_opacity || root_style.host_controls_root_background
            {
                with_root_style_overrides(
                    &source,
                    root_style.opacity,
                    normalize_zero_root_opacity,
                    root_style.host_controls_root_background,
                )?
            } else {
                source
            };
            let source = with_inherited_color(&source, root_style.inherited_color)?;
            let raster_opacity = if root_style.neutralize_root_opacity {
                1.0
            } else {
                root_style.opacity
            };
            let parsed_tree = (source != self.source)
                .then(|| parse_tree(&source))
                .transpose()?;
            let tree = parsed_tree.as_ref().unwrap_or(&self.tree);
            let neutralized_root_opacity = root_style
                .neutralize_root_opacity
                .then_some(tree.root().opacity().get());
            render_tree(
                tree,
                width,
                height,
                raster_opacity,
                neutralized_root_opacity,
                &mut pixels,
            )?;
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
    let tree = usvg::Tree::from_str(source, &options)
        .map_err(|error| SvgError::InvalidDocument(error.to_string()))?;
    if !tree.filters().is_empty() {
        return Err(SvgError::UnsupportedFilterEffects);
    }
    Ok(tree)
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

fn with_root_style_overrides(
    source: &str,
    inherited_root_opacity: f32,
    neutralize_root_opacity: bool,
    host_controls_root_background: bool,
) -> Result<String, SvgError> {
    let xml = roxmltree::Document::parse(source)
        .map_err(|error| SvgError::InvalidDocument(error.to_string()))?;
    let root = xml.root_element();
    let mut edits = Vec::<(Range<usize>, String)>::new();
    let mut inserted_root_attributes = String::new();

    let mut root_style_properties = Vec::new();
    let mut stylesheet_properties = Vec::new();
    if neutralize_root_opacity {
        root_style_properties.push("opacity");
    }
    if host_controls_root_background {
        root_style_properties.extend(["background-color", "background"]);
        stylesheet_properties.extend(["background-color", "background"]);
        for attribute in root.attributes().filter(|attribute| {
            attribute.name() == "background-color" || attribute.name() == "background"
        }) {
            edits.push((attribute.range(), String::new()));
        }
    }

    if !root_style_properties.is_empty() {
        let existing_style = root.attribute("style");
        let retained = existing_style.map_or_else(String::new, |style| {
            strip_inline_style_properties(style, &root_style_properties)
        });
        let style_value = if neutralize_root_opacity {
            append_inline_declarations(&retained, &format!("opacity:{inherited_root_opacity}"))
        } else {
            retained
        };
        if let Some(attribute) = root
            .attributes()
            .find(|attribute| attribute.name() == "style")
        {
            edits.push((
                attribute.range(),
                format!("style=\"{}\"", escape_xml_attribute(&style_value)),
            ));
        } else if neutralize_root_opacity {
            inserted_root_attributes.push_str(&format!(
                " style=\"{}\"",
                escape_xml_attribute(&style_value)
            ));
        }
    }

    let scope_attribute = unique_scope_attribute(source);
    let mut has_scoped_stylesheet_properties = false;
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
        let stylesheet_text = node
            .children()
            .filter(|child| child.is_text())
            .filter_map(|child| child.text())
            .collect::<String>();
        if stylesheet_text.is_empty() {
            continue;
        }
        let Some(rewritten) =
            scope_stylesheet_properties(&stylesheet_text, &scope_attribute, &stylesheet_properties)
        else {
            continue;
        };
        has_scoped_stylesheet_properties = true;
        let content_range = xml_element_content_range(source, node).ok_or_else(|| {
            SvgError::InvalidDocument("unterminated SVG style element".to_owned())
        })?;
        edits.push((content_range, escape_xml_text(&rewritten)));
    }

    if has_scoped_stylesheet_properties {
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

struct OpacityDeclaration {
    value: String,
    important: bool,
    inline_style: bool,
    specificity: [u8; 3],
    source_order: usize,
}

fn normalize_svg_opacity_cascade(source: &str) -> Result<String, SvgError> {
    // usvg resolves `inherit` while it builds its tree and copies the parent's
    // `!important` bit with the inherited value. Materialize each element's
    // winning declaration first, while keeping the literal `inherit` so it
    // still resolves in each `<use>` clone.
    let xml = roxmltree::Document::parse(source)
        .map_err(|error| SvgError::InvalidDocument(error.to_string()))?;
    let root = xml.root_element();
    let stylesheets = xml
        .descendants()
        .filter(|node| node.has_tag_name("style"))
        .filter_map(|node| {
            if node
                .attribute("type")
                .is_some_and(|style_type| style_type != "text/css")
            {
                return None;
            }
            // Match usvg's stylesheet loader, which reads only `node.text()`.
            let text = node.text()?.to_owned();
            (!text.is_empty()).then_some((node, text))
        })
        .collect::<Vec<_>>();

    let mut stylesheet = simplecss::StyleSheet::new();
    for (_, text) in &stylesheets {
        stylesheet.parse_more(text);
    }

    let mut edits = Vec::<(Range<usize>, String)>::new();
    for node in root.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().name() != "style"
            && node
                .tag_name()
                .namespace()
                .is_none_or(|namespace| namespace == SVG_NAMESPACE)
    }) {
        let winner = opacity_winner(node, &stylesheet);
        for attribute in node.attributes().filter(|attribute| {
            attribute.name() == "opacity"
                && matches!(
                    attribute.namespace(),
                    None | Some(SVG_NAMESPACE) | Some(XLINK_NAMESPACE) | Some(XML_NAMESPACE)
                )
        }) {
            edits.push((attribute.range(), String::new()));
        }

        let Some(winner) = winner else {
            continue;
        };
        let existing_style = node.attribute("style");
        let retained = existing_style.map_or_else(String::new, |style| {
            strip_inline_style_properties(style, &["opacity"])
        });
        let style_value =
            append_inline_declarations(&retained, &format!("opacity:{}", winner.value));
        if let Some(attribute) = node
            .attributes()
            .find(|attribute| attribute.name() == "style")
        {
            edits.push((
                attribute.range(),
                format!("style=\"{}\"", escape_xml_attribute(&style_value)),
            ));
        } else {
            let start = node.range().start;
            let end = root_start_tag_end(source, start).ok_or_else(|| {
                SvgError::InvalidDocument("unterminated SVG element tag".to_owned())
            })?;
            let insertion = if end > start && source.as_bytes()[end - 1] == b'/' {
                end - 1
            } else {
                end
            };
            edits.push((
                insertion..insertion,
                format!(" style=\"{}\"", escape_xml_attribute(&style_value)),
            ));
        }
    }

    for (node, text) in &stylesheets {
        let Some(rewritten) = strip_stylesheet_properties(text, &["opacity"]) else {
            continue;
        };
        let range = xml_element_content_range(source, *node).ok_or_else(|| {
            SvgError::InvalidDocument("unterminated SVG style element".to_owned())
        })?;
        edits.push((range, escape_xml_text(&rewritten)));
    }

    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut result = source.to_owned();
    for (range, replacement) in edits {
        result.replace_range(range, &replacement);
    }
    Ok(result)
}

fn opacity_winner(
    node: roxmltree::Node<'_, '_>,
    stylesheet: &simplecss::StyleSheet<'_>,
) -> Option<OpacityDeclaration> {
    let mut winner: Option<OpacityDeclaration> = None;

    let mut consider = |value: &str,
                        important: bool,
                        inline_style: bool,
                        specificity: [u8; 3],
                        source_order: usize| {
        let candidate = OpacityDeclaration {
            value: value.to_owned(),
            important,
            inline_style,
            specificity,
            source_order,
        };
        let cascade_key = |declaration: &OpacityDeclaration| {
            (
                declaration.important,
                declaration.inline_style,
                declaration.specificity,
                declaration.source_order,
            )
        };
        if winner
            .as_ref()
            .is_none_or(|current| cascade_key(&candidate) > cascade_key(current))
        {
            winner = Some(candidate);
        }
    };

    let mut source_order = 0;
    for attribute in node.attributes().filter(|attribute| {
        attribute.name() == "opacity"
            && matches!(
                attribute.namespace(),
                None | Some(SVG_NAMESPACE) | Some(XLINK_NAMESPACE) | Some(XML_NAMESPACE)
            )
    }) {
        consider(attribute.value(), false, false, [0, 0, 0], source_order);
        source_order += 1;
    }

    let element = SvgCssElement(node);
    // simplecss sorts by specificity and preserves source order for ties.
    for rule in &stylesheet.rules {
        let specificity = rule.selector.specificity();
        let matches = rule.selector.matches(&element);
        for declaration in &rule.declarations {
            if matches && declaration.name == "opacity" {
                consider(
                    declaration.value,
                    declaration.important,
                    false,
                    specificity,
                    source_order,
                );
            }
            source_order += 1;
        }
    }

    if let Some(style) = node.attribute("style") {
        for declaration in simplecss::DeclarationTokenizer::from(style) {
            if declaration.name == "opacity" {
                consider(
                    declaration.value,
                    declaration.important,
                    true,
                    [0, 0, 0],
                    source_order,
                );
            }
            source_order += 1;
        }
    }

    winner
}

struct SvgCssElement<'a, 'input: 'a>(roxmltree::Node<'a, 'input>);

impl simplecss::Element for SvgCssElement<'_, '_> {
    fn parent_element(&self) -> Option<Self> {
        self.0.parent_element().map(SvgCssElement)
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        self.0.prev_sibling_element().map(SvgCssElement)
    }

    fn has_local_name(&self, local_name: &str) -> bool {
        self.0.tag_name().name() == local_name
    }

    fn attribute_matches(
        &self,
        local_name: &str,
        operator: simplecss::AttributeOperator<'_>,
    ) -> bool {
        self.0
            .attribute(local_name)
            .is_some_and(|value| operator.matches(value))
    }

    fn pseudo_class_matches(&self, class: simplecss::PseudoClass<'_>) -> bool {
        match class {
            simplecss::PseudoClass::FirstChild => self.prev_sibling_element().is_none(),
            _ => false,
        }
    }
}

struct ParsedCssDeclaration {
    name: String,
    raw: String,
    range: Range<usize>,
}

struct ScopedStylesheetParser<'a> {
    source: &'a str,
    scope_attribute: Option<&'a str>,
    properties: &'a [&'a str],
    removals: Vec<Range<usize>>,
    scoped_rules: Vec<String>,
}

struct CssDeclarationSourceParser<'a> {
    source: &'a str,
}

impl<'i> cssparser::DeclarationParser<'i> for CssDeclarationSourceParser<'_> {
    type Declaration = ParsedCssDeclaration;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: cssparser::CowRcStr<'i>,
        input: &mut cssparser::Parser<'i, 't>,
        declaration_start: &cssparser::ParserState,
    ) -> Result<Self::Declaration, cssparser::ParseError<'i, Self::Error>> {
        while input.next_including_whitespace_and_comments().is_ok() {}
        let raw = input.slice(declaration_start.position()..input.position());
        let start = source_slice_offset(self.source, raw);
        Ok(ParsedCssDeclaration {
            name: name.to_string(),
            raw: raw.to_owned(),
            range: start..start + raw.len(),
        })
    }
}

impl<'i> cssparser::AtRuleParser<'i> for CssDeclarationSourceParser<'_> {
    type Prelude = ();
    type AtRule = ParsedCssDeclaration;
    type Error = ();
}

impl<'i> cssparser::QualifiedRuleParser<'i> for CssDeclarationSourceParser<'_> {
    type Prelude = ();
    type QualifiedRule = ParsedCssDeclaration;
    type Error = ();
}

impl<'i> cssparser::RuleBodyItemParser<'i, ParsedCssDeclaration, ()>
    for CssDeclarationSourceParser<'_>
{
    fn parse_qualified(&self) -> bool {
        false
    }

    fn parse_declarations(&self) -> bool {
        true
    }
}

impl<'i> cssparser::AtRuleParser<'i> for ScopedStylesheetParser<'_> {
    type Prelude = ();
    type AtRule = ();
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
        while input.next().is_ok() {}
        Ok(())
    }
}

impl<'i> cssparser::QualifiedRuleParser<'i> for ScopedStylesheetParser<'_> {
    type Prelude = String;
    type QualifiedRule = ();
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut cssparser::Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, Self::Error>> {
        let start = input.position();
        while input.next().is_ok() {}
        Ok(input.slice(start..input.position()).trim().to_owned())
    }

    fn parse_block<'t>(
        &mut self,
        selector_list: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut cssparser::Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, cssparser::ParseError<'i, Self::Error>> {
        let mut declaration_parser = CssDeclarationSourceParser {
            source: self.source,
        };
        let declarations = cssparser::RuleBodyParser::new(input, &mut declaration_parser)
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        let scoped_declarations = declarations
            .iter()
            .filter(|declaration| {
                self.properties
                    .iter()
                    .any(|property| declaration.name.eq_ignore_ascii_case(property))
            })
            .collect::<Vec<_>>();
        if scoped_declarations.is_empty() {
            return Ok(());
        }

        self.removals
            .extend(scoped_declarations.iter().map(|declaration| {
                let mut range = declaration.range.clone();
                while self
                    .source
                    .as_bytes()
                    .get(range.end)
                    .is_some_and(u8::is_ascii_whitespace)
                {
                    range.end += 1;
                }
                if self.source.as_bytes().get(range.end) == Some(&b';') {
                    range.end += 1;
                }
                range
            }));
        let raw_declarations = scoped_declarations
            .iter()
            .map(|declaration| declaration.raw.clone())
            .collect::<Vec<_>>();
        if let Some(scope_attribute) = self.scope_attribute {
            for selector in split_css_selector_list(&selector_list) {
                let selector = selector.trim();
                if selector.is_empty() {
                    continue;
                }
                if let Some(rule) =
                    scoped_property_rule(selector, scope_attribute, &raw_declarations)
                {
                    self.scoped_rules.push(rule);
                }
            }
        }
        Ok(())
    }
}

fn scoped_property_rule(
    selector: &str,
    scope_attribute: &str,
    declarations: &[String],
) -> Option<String> {
    let insertion = selector_scope_insertion(selector);
    if insertion == 0 {
        return None;
    }
    let mut scoped_rule = String::with_capacity(selector.len() + scope_attribute.len() + 16);
    scoped_rule.push_str(&selector[..insertion]);
    scoped_rule.push('[');
    scoped_rule.push_str(scope_attribute);
    scoped_rule.push(']');
    scoped_rule.push_str(&selector[insertion..]);
    scoped_rule.push_str(" {");
    for declaration in declarations {
        scoped_rule.push(' ');
        scoped_rule.push_str(declaration);
        scoped_rule.push(';');
    }
    scoped_rule.push_str(" }");
    Some(scoped_rule)
}

fn scope_stylesheet_properties(
    source: &str,
    scope_attribute: &str,
    properties: &[&str],
) -> Option<String> {
    rewrite_stylesheet_properties(source, Some(scope_attribute), properties)
}

fn strip_stylesheet_properties(source: &str, properties: &[&str]) -> Option<String> {
    rewrite_stylesheet_properties(source, None, properties)
}

fn rewrite_stylesheet_properties(
    source: &str,
    scope_attribute: Option<&str>,
    properties: &[&str],
) -> Option<String> {
    let mut parser_state = ScopedStylesheetParser {
        source,
        scope_attribute,
        properties,
        removals: Vec::new(),
        scoped_rules: Vec::new(),
    };
    let mut input = cssparser::ParserInput::new(source);
    let mut parser = cssparser::Parser::new(&mut input);
    for _ in cssparser::StyleSheetParser::new(&mut parser, &mut parser_state).filter_map(Result::ok)
    {
    }
    if parser_state.removals.is_empty()
        || (scope_attribute.is_some() && parser_state.scoped_rules.is_empty())
    {
        return None;
    }

    parser_state
        .removals
        .sort_by_key(|range| std::cmp::Reverse(range.start));
    let mut rewritten = source.to_owned();
    for range in parser_state.removals {
        rewritten.replace_range(range, "");
    }
    for scoped_rule in parser_state.scoped_rules {
        rewritten.push('\n');
        rewritten.push_str(&scoped_rule);
    }
    Some(rewritten)
}

fn source_slice_offset(source: &str, slice: &str) -> usize {
    let source_start = source.as_ptr() as usize;
    let slice_start = slice.as_ptr() as usize;
    let offset = slice_start
        .checked_sub(source_start)
        .expect("CSS parser slices come from their source buffer");
    debug_assert!(source.is_char_boundary(offset));
    offset
}

fn split_css_selector_list(selectors: &str) -> Vec<&str> {
    let bytes = selectors.as_bytes();
    let mut result = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let mut bracket_depth = 0_u32;
    let mut parenthesis_depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    let mut in_comment = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if in_comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                in_comment = false;
                index += 2;
                continue;
            }
            index += 1;
            continue;
        }
        if let Some(quote_byte) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == quote_byte {
                quote = None;
            }
            index += 1;
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            in_comment = true;
            index += 2;
            continue;
        }
        if byte == b'\\' {
            index += 2;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'(' => parenthesis_depth += 1,
            b')' => parenthesis_depth = parenthesis_depth.saturating_sub(1),
            b',' if bracket_depth == 0 && parenthesis_depth == 0 => {
                result.push(&selectors[start..index]);
                start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }
    result.push(&selectors[start..]);
    result
}

fn selector_scope_insertion(selector: &str) -> usize {
    let bytes = selector.as_bytes();
    let mut index = 0;
    let mut last_significant_end = 0;
    let mut bracket_depth = 0_u32;
    let mut parenthesis_depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    let mut in_comment = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if in_comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                in_comment = false;
                index += 2;
                continue;
            }
            index += 1;
            continue;
        }
        if let Some(quote_byte) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == quote_byte {
                quote = None;
            }
            index += 1;
            last_significant_end = index;
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            in_comment = true;
            index += 2;
            continue;
        }
        if byte == b'\\' {
            index += 1;
            if index < bytes.len() {
                let escaped_char_len = selector[index..].chars().next().map_or(0, char::len_utf8);
                index += escaped_char_len;
            }
            last_significant_end = index;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'(' => parenthesis_depth += 1,
            b')' => parenthesis_depth = parenthesis_depth.saturating_sub(1),
            _ => {}
        }
        if !byte.is_ascii_whitespace() {
            last_significant_end = index + 1;
        }
        index += 1;
    }

    debug_assert!(selector.is_char_boundary(last_significant_end));
    last_significant_end
}

fn xml_element_content_range(source: &str, node: roxmltree::Node<'_, '_>) -> Option<Range<usize>> {
    let element_range = node.range();
    let start_tag_end = root_start_tag_end(source, element_range.start)?;
    let content_start = start_tag_end.checked_add(1)?;
    let closing_tag_start = source[..element_range.end].rfind("</")?;
    (content_start <= closing_tag_start).then_some(content_start..closing_tag_start)
}

fn unique_scope_attribute(source: &str) -> String {
    let base = "data-raikiri-root-opacity-scope";
    (0_u32..)
        .map(|suffix| {
            if suffix == 0 {
                base.to_owned()
            } else {
                format!("{base}-{suffix}")
            }
        })
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
    neutralized_root_opacity: Option<f32>,
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

    if let Some(root_opacity) = neutralized_root_opacity.filter(|opacity| *opacity < 1.0) {
        // Keep the host value available while parsing so explicit `inherit`
        // declarations (including nodes expanded from `<use>`) resolve in
        // their original context. Remove the resulting SVG root group alpha
        // here, while pixels are premultiplied, because the host composites
        // that opacity around the raster and its box decorations together.
        let inverse_opacity = 1.0 / root_opacity;
        for channel in pixmap.data_mut() {
            *channel = (f32::from(*channel) * inverse_opacity)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }

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
