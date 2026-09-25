//! Static SVG parsing and bounded rasterization.
//!
//! Backend types stay private to this crate. SVG references that could read
//! local files, issue requests, or recursively load data URLs are rejected.

use raikiri_traits::DecodedImage;

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
    /// Whether the host computed visibility permits painting.
    pub visible: bool,
}

impl Default for SvgRootStyle {
    fn default() -> Self {
        Self {
            inherited_color: [0, 0, 0, 255],
            opacity: 1.0,
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

        let intrinsic = intrinsic_size(root);
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
            let tree = if root_style.inherited_color == [0, 0, 0, 255] {
                &self.tree
            } else {
                let source = with_inherited_color(&self.source, root_style.inherited_color)?;
                if source == self.source {
                    &self.tree
                } else {
                    let tree = parse_tree(&source)?;
                    render_tree(&tree, width, height, root_style.opacity, &mut pixels)?;
                    return Ok(DecodedImage {
                        width,
                        height,
                        rgba: pixels,
                    });
                }
            };

            render_tree(tree, width, height, root_style.opacity, &mut pixels)?;
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
