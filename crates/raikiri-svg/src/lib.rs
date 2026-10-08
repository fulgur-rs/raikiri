//! Static SVG parsing and bounded rasterization.
//!
//! Backend types stay private to this crate. SVG references that could read
//! local files, issue requests, or recursively load data URLs are rejected.

use raikiri_traits::DecodedImage;
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
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

/// The concrete CSS viewport used to prepare or rasterize an SVG.
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
    /// Host computed opacity. It is assigned to the SVG root while inherited
    /// opacity declarations are resolved. If root opacity is neutralized, the
    /// caller must composite this opacity around the raster and its box
    /// decorations together. The raster backend removes root opacity after its
    /// 8-bit pass, so small rounding differences are possible. Otherwise this
    /// opacity is applied once to the completed raster.
    pub opacity: f32,
    /// Remove the SVG root group's opacity from the raster when the caller
    /// composites the computed opacity around the SVG and its box decorations.
    /// Source-only preparation retains this opacity for inheritance; see
    /// [`SvgDocument::styled_source`].
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

/// Resolved font properties inherited by a standalone SVG root.
#[derive(Debug, Clone, Copy)]
pub struct SvgRootFont<'a> {
    /// Finite non-negative font size in CSS pixels.
    pub size: f32,
    /// CSS family candidate list, preserving all fallback names.
    pub family: &'a str,
    /// Finite absolute weight in the inclusive range 1 to 1000.
    pub weight: f32,
    /// CSS `font-style` keyword: `normal`, `italic` or `oblique`.
    pub style: &'a str,
}

/// Resolved declarations for one descendant of the original SVG source.
#[derive(Debug, Clone, Copy)]
pub struct SvgElementStyle<'a> {
    /// Zero-based element preorder index, counting the root as index zero.
    /// The root is styled separately and must not be included here.
    pub element_index: usize,
    /// Resolved CSS declarations for color, display, opacity, visibility and
    /// font size, family, weight or style. Preserve literal `inherit` when
    /// a declaration must inherit through an instantiated `use` subtree.
    pub declarations: &'a str,
}

#[derive(Clone, Copy, Default)]
struct SourceRootStyle<'a> {
    color: Option<[u8; 4]>,
    font: Option<(f32, &'a str)>,
    font_face: Option<(f32, &'a str)>,
    visible: Option<bool>,
    elements: &'a [SvgElementStyle<'a>],
}

/// SVG parsing, viewport, or allocation failure.
#[derive(Debug)]
#[non_exhaustive]
pub enum SvgError {
    /// The source is not a supported SVG document.
    InvalidDocument(String),
    /// The document contains a DTD other than the inert SVG 1.1 public declaration.
    UnsupportedDoctype,
    /// An SVG resource reference would access a resource outside this file.
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
                f.write_str("SVG resource references outside the document are disabled")
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
    root_has_color: bool,
}

impl SvgDocument {
    /// Parses and validates a UTF-8 SVG document.
    pub fn parse(data: &[u8]) -> Result<Self, SvgError> {
        let source = std::str::from_utf8(data)
            .map_err(|error| SvgError::InvalidDocument(error.to_string()))?;
        let normalized = without_svg11_public_doctype(source)?;
        ensure_initial_svg_xml_depth_bounded(&normalized)?;
        // Validate placement in the original XML, then keep DTD-free source
        // for every subsequent parser pass. No DTD resource is ever loaded.
        let xml = roxmltree::Document::parse_with_options(
            source,
            roxmltree::ParsingOptions {
                allow_dtd: matches!(normalized, std::borrow::Cow::Owned(_)),
                ..Default::default()
            },
        )
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
        let root_has_color = root.attribute("color").is_some();
        preflight_initial_svg_selectors(&xml)?;
        let tree = parse_tree(&normalized)?;
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
            source: normalized.into_owned(),
            intrinsic,
            has_view_box,
            root_has_color,
        })
    }

    /// Returns the source's natural dimensions and ratio without CSS defaults.
    pub fn intrinsic_size(&self) -> SvgIntrinsicSize {
        self.intrinsic
    }

    /// Prepares SVG source for the resolved CSS viewport and host style.
    ///
    /// No pixels are allocated. When `neutralize_root_opacity` is set, the
    /// source retains the host's root opacity so an SVG parser can resolve
    /// explicit `inherit` values. A vector consumer must remove only the
    /// resolved root group's opacity after parsing, then composite the host
    /// box and SVG together with that opacity.
    /// `root_style.visible` controls rasterization only; a vector consumer
    /// handles the host's visibility before drawing this source.
    /// External resource references, CSS imports and XML stylesheet processing
    /// instructions are rejected before source is exported. SVG navigation
    /// links and same-document fragment references remain available.
    /// CSS at-rules are retained without interpretation; the vector consumer
    /// determines which of them it supports.
    pub fn styled_source(
        &self,
        viewport: SvgViewport,
        root_style: SvgRootStyle,
    ) -> Result<String, SvgError> {
        self.styled_source_impl(viewport, root_style, SourceRootStyle::default())
    }

    /// Prepares source with the host cascade's effective SVG root color.
    ///
    /// Resolve root presentation attributes and author CSS before passing
    /// `root_color`. Selector matches are frozen before replacing root color
    /// declarations; descendant colors and explicit inheritance are preserved.
    /// Other preparation rules match [`Self::styled_source`].
    pub fn styled_source_with_root_color(
        &self,
        viewport: SvgViewport,
        root_style: SvgRootStyle,
        root_color: [u8; 4],
    ) -> Result<String, SvgError> {
        self.styled_source_impl(
            viewport,
            root_style,
            SourceRootStyle {
                color: Some(root_color),
                ..SourceRootStyle::default()
            },
        )
    }

    /// Prepares source with resolved root color and font inheritance.
    ///
    /// `font_size` is a finite non-negative CSS pixel size; `font_family` is a
    /// CSS family list. Resolve presentation attributes and author CSS first.
    /// Original selector matches and descendant font declarations are preserved.
    pub fn styled_source_with_root_color_and_font(
        &self,
        viewport: SvgViewport,
        root_style: SvgRootStyle,
        root_color: [u8; 4],
        font_size: f32,
        font_family: &str,
    ) -> Result<String, SvgError> {
        if !valid_root_font(font_size, font_family) {
            return Err(SvgError::InvalidDocument(
                "invalid SVG root font style".into(),
            ));
        }
        self.styled_source_impl(
            viewport,
            root_style,
            SourceRootStyle {
                color: Some(root_color),
                font: Some((font_size, font_family)),
                ..SourceRootStyle::default()
            },
        )
    }

    /// Prepares source with the host's resolved root color, font and visibility.
    ///
    /// Resolve presentation attributes and author CSS before calling this
    /// method. Original selector matches are frozen before root overrides.
    /// Descendant declarations, including explicit visibility and font
    /// overrides, are preserved. Unlike [`Self::styled_source`], this method
    /// applies `root_style.visible` to the source root, so a hidden root can
    /// still contain visible descendants.
    pub fn styled_source_with_resolved_root_style(
        &self,
        viewport: SvgViewport,
        root_style: SvgRootStyle,
        root_color: [u8; 4],
        font: SvgRootFont<'_>,
    ) -> Result<String, SvgError> {
        self.styled_source_with_resolved_styles(viewport, root_style, root_color, font, &[])
    }

    /// Prepares source with host-cascaded root and descendant declarations.
    ///
    /// Element indices refer to the original source before selector freezing.
    /// Existing declarations for an overridden property are scoped away from
    /// that element; other declarations and original selector matches survive.
    /// Declaration parsing, generated source and selector work share the
    /// ordinary source preparation budget. External references are rejected.
    pub fn styled_source_with_resolved_styles(
        &self,
        viewport: SvgViewport,
        root_style: SvgRootStyle,
        root_color: [u8; 4],
        font: SvgRootFont<'_>,
        elements: &[SvgElementStyle<'_>],
    ) -> Result<String, SvgError> {
        if !valid_root_font(font.size, font.family)
            || !font.weight.is_finite()
            || !(1.0..=1000.0).contains(&font.weight)
            || !["normal", "italic", "oblique"]
                .iter()
                .any(|keyword| font.style.eq_ignore_ascii_case(keyword))
        {
            return Err(SvgError::InvalidDocument(
                "invalid SVG root font style".into(),
            ));
        }
        self.styled_source_impl(
            viewport,
            root_style,
            SourceRootStyle {
                color: Some(root_color),
                font: Some((font.size, font.family)),
                font_face: Some((font.weight, font.style)),
                visible: Some(root_style.visible),
                elements,
            },
        )
    }

    fn styled_source_impl(
        &self,
        viewport: SvgViewport,
        root_style: SvgRootStyle,
        source_style: SourceRootStyle<'_>,
    ) -> Result<String, SvgError> {
        if !viewport.width.is_finite()
            || !viewport.height.is_finite()
            || viewport.width <= 0.0
            || viewport.height <= 0.0
        {
            return Err(SvgError::InvalidViewport);
        }
        if !root_style.opacity.is_finite() || !(0.0..=1.0).contains(&root_style.opacity) {
            return Err(SvgError::InvalidOpacity);
        }
        reject_exported_external_references(&self.source)?;
        let source = self.prepare_source(viewport, root_style, source_style, true)?;
        if !source_style.elements.is_empty() {
            reject_exported_external_references(&source)?;
        }
        Ok(source)
    }

    fn prepare_source(
        &self,
        viewport: SvgViewport,
        root_style: SvgRootStyle,
        source_style: SourceRootStyle<'_>,
        exact_viewport: bool,
    ) -> Result<String, SvgError> {
        let viewport_matches = viewport_matches_tree(
            &self.tree,
            viewport.width,
            viewport.height,
            self.has_view_box && !exact_viewport,
        );
        let modifies_source = exact_viewport
            || !viewport_matches
            || source_style.color.is_some()
            || source_style.font.is_some()
            || source_style.visible.is_some()
            || root_style.neutralize_root_opacity
            || root_style.host_controls_root_background
            || !self.root_has_color;
        // Freeze matches before rewriting attributes inspected by selectors.
        let mut rewrite_budget = SelectorFreezeBudget::new();
        let source = if modifies_source {
            freeze_svg_stylesheet_selectors(&self.source, &mut rewrite_budget)?
        } else {
            self.source.clone()
        };
        let source = if exact_viewport {
            normalize_svg_css_types(&source, &mut rewrite_budget)?.unwrap_or(source)
        } else {
            source
        };
        let source = if source_style.font.is_some() {
            normalize_svg_font_shorthands(&source, &mut rewrite_budget)?
        } else {
            source
        };
        let source = if source_style.elements.is_empty() {
            source
        } else {
            with_element_style_overrides(&source, source_style.elements, &mut rewrite_budget)?
        };
        let source = if viewport_matches {
            source
        } else {
            with_root_viewport_size(&source, viewport.width, viewport.height)?
        };
        let source = if root_style.neutralize_root_opacity {
            normalize_svg_opacity_cascade(&source, &mut rewrite_budget)?
        } else {
            source
        };
        let source = if root_style.neutralize_root_opacity
            || root_style.host_controls_root_background
            || source_style.color.is_some()
            || source_style.font.is_some()
            || source_style.visible.is_some()
        {
            with_root_style_overrides(
                &source,
                root_style.opacity,
                root_style.neutralize_root_opacity,
                root_style.host_controls_root_background || root_style.neutralize_root_opacity,
                source_style,
                &mut rewrite_budget,
            )?
        } else {
            source
        };
        with_inherited_color(&source, root_style.inherited_color)
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
        self.rasterize_inner(viewport, root_style, max_output_bytes, false)
    }

    /// Rasterizes inline SVG on the CSS pixel grid without stretching the
    /// rounded backing buffer. The caller must paint at one image pixel per
    /// CSS pixel and clip to the original fractional viewport.
    ///
    /// Uses the same opacity controls and allocation limit as [`Self::rasterize`].
    pub fn rasterize_at_css_pixel_scale(
        &self,
        viewport: SvgViewport,
        root_style: SvgRootStyle,
        max_output_bytes: Option<u64>,
    ) -> Result<DecodedImage, SvgError> {
        self.rasterize_inner(viewport, root_style, max_output_bytes, true)
    }

    fn rasterize_inner(
        &self,
        viewport: SvgViewport,
        root_style: SvgRootStyle,
        max_output_bytes: Option<u64>,
        css_pixel_scale: bool,
    ) -> Result<DecodedImage, SvgError> {
        if !root_style.opacity.is_finite() || !(0.0..=1.0).contains(&root_style.opacity) {
            return Err(SvgError::InvalidOpacity);
        }

        let (width, height, byte_len) = checked_output_size(viewport, max_output_bytes)?;
        let mut pixels = allocate_transparent_pixels(byte_len)?;
        let viewport = if css_pixel_scale {
            viewport
        } else {
            SvgViewport {
                width: width as f32,
                height: height as f32,
            }
        };

        if root_style.visible && root_style.opacity > 0.0 {
            let source =
                self.prepare_source(viewport, root_style, SourceRootStyle::default(), false)?;
            let raster_opacity = if root_style.neutralize_root_opacity {
                1.0
            } else {
                root_style.opacity
            };
            let parsed_tree = (source != self.source)
                .then(|| parse_tree(&source))
                .transpose()?;
            let tree = parsed_tree.as_ref().unwrap_or(&self.tree);
            render_tree(
                tree,
                width,
                height,
                viewport,
                raster_opacity,
                root_style
                    .neutralize_root_opacity
                    .then_some(root_style.opacity),
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

#[derive(Default)]
struct InitialStylesheetResourceEstimate {
    selector_count: usize,
    selector_ast_bytes: usize,
    declaration_storage_bytes: usize,
}

// SimpleCSS may recover after malformed tokens, so fallback estimates must retain the scanned prefix.
enum SimpleCssRecoveryPrefix {
    SelectorHeader {
        selector_start: usize,
        selector_count: usize,
        source_bytes: usize,
    },
    DeclarationBody {
        selector_count: usize,
        declaration_count: usize,
        declarations_start: usize,
    },
}

fn estimate_initial_stylesheet_resources(
    stylesheet: &str,
) -> Result<InitialStylesheetResourceEstimate, SvgError> {
    let bytes = stylesheet.as_bytes();
    let mut cursor = 0;
    let mut estimate = InitialStylesheetResourceEstimate::default();

    while skip_simplecss_spaces_and_comments(bytes, &mut cursor) {
        if cursor == bytes.len() {
            break;
        }

        if bytes[cursor] == b'@' && is_simplecss_name_start(bytes.get(cursor + 1).copied()) {
            cursor = skip_simplecss_at_rule(bytes, cursor);
            continue;
        }

        let selector_start = cursor;
        let mut selector_count = 1usize;
        let mut selector_source_bytes = 0usize;
        loop {
            if cursor >= bytes.len() {
                break;
            }

            if bytes[cursor..].starts_with(b"/*") {
                if let Some(comment_end) = simplecss_comment_end(bytes, cursor) {
                    if simplecss_has_block_delimiter(&bytes[cursor..comment_end]) {
                        return include_simplecss_recovery_suffix(
                            estimate,
                            bytes,
                            cursor,
                            SimpleCssRecoveryPrefix::SelectorHeader {
                                selector_start,
                                selector_count,
                                source_bytes: selector_source_bytes,
                            },
                        );
                    }
                    cursor = comment_end;
                    continue;
                }
                if simplecss_has_block_delimiter(&bytes[cursor..]) {
                    return include_simplecss_recovery_suffix(
                        estimate,
                        bytes,
                        cursor,
                        SimpleCssRecoveryPrefix::SelectorHeader {
                            selector_start,
                            selector_count,
                            source_bytes: selector_source_bytes,
                        },
                    );
                }
                cursor = bytes.len();
                break;
            }

            match bytes[cursor] {
                b'\'' | b'"' => {
                    let string_end = simplecss_string_end(bytes, cursor);
                    if simplecss_has_block_delimiter(&bytes[cursor..string_end]) {
                        return include_simplecss_recovery_suffix(
                            estimate,
                            bytes,
                            cursor,
                            SimpleCssRecoveryPrefix::SelectorHeader {
                                selector_start,
                                selector_count,
                                source_bytes: selector_source_bytes,
                            },
                        );
                    }
                    selector_source_bytes = selector_source_bytes
                        .checked_add(string_end - cursor)
                        .ok_or_else(initial_parse_selector_limit_error)?;
                    cursor = string_end;
                }
                b'(' => {
                    let function_end = simplecss_function_end(bytes, cursor);
                    if simplecss_has_block_delimiter(&bytes[cursor..function_end]) {
                        return include_simplecss_recovery_suffix(
                            estimate,
                            bytes,
                            cursor,
                            SimpleCssRecoveryPrefix::SelectorHeader {
                                selector_start,
                                selector_count,
                                source_bytes: selector_source_bytes,
                            },
                        );
                    }
                    selector_source_bytes = selector_source_bytes
                        .checked_add(function_end - cursor)
                        .ok_or_else(initial_parse_selector_limit_error)?;
                    cursor = function_end;
                }
                b',' => {
                    selector_count = selector_count
                        .checked_add(1)
                        .ok_or_else(initial_parse_selector_limit_error)?;
                    selector_source_bytes = selector_source_bytes
                        .checked_add(1)
                        .ok_or_else(initial_parse_selector_limit_error)?;
                    cursor += 1;
                }
                b'{' => break,
                _ => {
                    selector_source_bytes = selector_source_bytes
                        .checked_add(1)
                        .ok_or_else(initial_parse_selector_limit_error)?;
                    cursor += 1;
                }
            }
        }

        estimate.selector_count = estimate
            .selector_count
            .checked_add(selector_count)
            .ok_or_else(initial_parse_selector_limit_error)?;
        let selector_headers = selector_count
            .checked_mul(std::mem::size_of::<simplecss::Rule<'static>>())
            .and_then(|bytes| bytes.checked_mul(2))
            .ok_or_else(initial_parse_selector_limit_error)?;
        let selector_nodes = selector_source_bytes
            .checked_mul(MAX_INITIAL_PARSE_SELECTOR_AST_BYTES_PER_SOURCE_BYTE)
            .ok_or_else(initial_parse_selector_limit_error)?;
        estimate.selector_ast_bytes = estimate
            .selector_ast_bytes
            .checked_add(selector_headers)
            .and_then(|bytes| bytes.checked_add(selector_nodes))
            .ok_or_else(initial_parse_selector_limit_error)?;

        if bytes.get(cursor) != Some(&b'{') {
            break;
        }
        cursor += 1;

        let declarations_start = cursor;
        let mut declaration_count = 1usize;
        let mut current_declaration_has_content = false;
        let mut nested_blocks = 0usize;
        while cursor < bytes.len() {
            if bytes[cursor..].starts_with(b"/*") {
                if let Some(comment_end) = simplecss_comment_end(bytes, cursor) {
                    if simplecss_has_block_delimiter(&bytes[cursor..comment_end]) {
                        return include_simplecss_recovery_suffix(
                            estimate,
                            bytes,
                            cursor,
                            SimpleCssRecoveryPrefix::DeclarationBody {
                                selector_count,
                                declaration_count,
                                declarations_start,
                            },
                        );
                    }
                    cursor = comment_end;
                    continue;
                }
                if simplecss_has_block_delimiter(&bytes[cursor..]) {
                    return include_simplecss_recovery_suffix(
                        estimate,
                        bytes,
                        cursor,
                        SimpleCssRecoveryPrefix::DeclarationBody {
                            selector_count,
                            declaration_count,
                            declarations_start,
                        },
                    );
                }
                cursor = bytes.len();
                break;
            }

            match bytes[cursor] {
                b'\'' | b'"' => {
                    let string_end = simplecss_string_end(bytes, cursor);
                    if simplecss_has_block_delimiter(&bytes[cursor..string_end]) {
                        return include_simplecss_recovery_suffix(
                            estimate,
                            bytes,
                            cursor,
                            SimpleCssRecoveryPrefix::DeclarationBody {
                                selector_count,
                                declaration_count,
                                declarations_start,
                            },
                        );
                    }
                    current_declaration_has_content = true;
                    cursor = string_end;
                }
                b'(' => {
                    let function_end = simplecss_function_end(bytes, cursor);
                    if simplecss_has_block_delimiter(&bytes[cursor..function_end]) {
                        return include_simplecss_recovery_suffix(
                            estimate,
                            bytes,
                            cursor,
                            SimpleCssRecoveryPrefix::DeclarationBody {
                                selector_count,
                                declaration_count,
                                declarations_start,
                            },
                        );
                    }
                    current_declaration_has_content = true;
                    cursor = function_end;
                }
                b'{' => {
                    current_declaration_has_content = true;
                    nested_blocks = nested_blocks
                        .checked_add(1)
                        .ok_or_else(initial_parse_selector_limit_error)?;
                    cursor += 1;
                }
                b'}' if nested_blocks == 0 => {
                    cursor += 1;
                    break;
                }
                b'}' => {
                    current_declaration_has_content = true;
                    nested_blocks -= 1;
                    cursor += 1;
                }
                b';' => {
                    if current_declaration_has_content {
                        declaration_count = declaration_count
                            .checked_add(1)
                            .ok_or_else(initial_parse_selector_limit_error)?;
                    } // cov:ignore: LLVM maps this block terminator without a counter.
                    current_declaration_has_content = false;
                    cursor += 1;
                }
                byte => {
                    if !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0C') {
                        current_declaration_has_content = true;
                    }
                    cursor += 1;
                }
            }
        }

        let declaration_vectors = selector_count
            .checked_add(1)
            .ok_or_else(initial_parse_selector_limit_error)?;
        let declaration_storage = declaration_vectors
            .checked_mul(declaration_count)
            .and_then(|count| {
                count.checked_mul(std::mem::size_of::<simplecss::Declaration<'static>>())
            })
            .and_then(|bytes| bytes.checked_mul(INITIAL_PARSE_DECLARATION_CAPACITY_FACTOR))
            .ok_or_else(initial_parse_selector_limit_error)?;
        estimate.declaration_storage_bytes = estimate
            .declaration_storage_bytes
            .checked_add(declaration_storage)
            .ok_or_else(initial_parse_selector_limit_error)?;
    }

    Ok(estimate)
}

fn include_simplecss_recovery_suffix(
    mut estimate: InitialStylesheetResourceEstimate,
    stylesheet: &[u8],
    start: usize,
    prefix: SimpleCssRecoveryPrefix,
) -> Result<InitialStylesheetResourceEstimate, SvgError> {
    let suffix = &stylesheet[start..];
    let starts_in_selector_header =
        matches!(&prefix, SimpleCssRecoveryPrefix::SelectorHeader { .. });
    let requires_raw_recovery = match &prefix {
        SimpleCssRecoveryPrefix::SelectorHeader { selector_start, .. } => {
            simplecss_recovery_selector_header_requires_raw_recovery(stylesheet, *selector_start)
                .unwrap_or(true)
        }
        SimpleCssRecoveryPrefix::DeclarationBody {
            declarations_start, ..
        } => simplecss_recovery_rule_body_requires_raw_recovery(stylesheet, *declarations_start)
            .unwrap_or(true),
    };
    let (suffix_selector_count, suffix_declaration_count) = if requires_raw_recovery {
        simplecss_raw_recovery_delimiter_counts(suffix)
    } else {
        simplecss_recovery_delimiter_counts(suffix, starts_in_selector_header)
    }
    .or_else(|| simplecss_raw_recovery_delimiter_counts(suffix))
    .ok_or_else(initial_parse_selector_limit_error)?;

    let (
        prefix_selector_count,
        prefix_source_bytes,
        current_rule_selector_count,
        current_rule_declaration_count,
    ) = match prefix {
        SimpleCssRecoveryPrefix::SelectorHeader {
            selector_start: _,
            selector_count,
            source_bytes,
        } => (selector_count, source_bytes, 0, 0),
        SimpleCssRecoveryPrefix::DeclarationBody {
            selector_count,
            declaration_count,
            declarations_start: _,
        } => (0, 0, selector_count, declaration_count),
    };
    let selector_count = prefix_selector_count
        .checked_add(suffix_selector_count)
        .ok_or_else(initial_parse_selector_limit_error)?;
    let selector_source_bytes = prefix_source_bytes
        .checked_add(suffix.len())
        .ok_or_else(initial_parse_selector_limit_error)?;
    let storage_selector_count = selector_count
        .checked_add(current_rule_selector_count)
        .ok_or_else(initial_parse_selector_limit_error)?;

    let selector_headers = selector_count
        .checked_mul(std::mem::size_of::<simplecss::Rule<'static>>())
        .and_then(|bytes| bytes.checked_mul(2))
        .ok_or_else(initial_parse_selector_limit_error)?;
    let selector_nodes = selector_source_bytes
        .checked_mul(MAX_INITIAL_PARSE_SELECTOR_AST_BYTES_PER_SOURCE_BYTE)
        .ok_or_else(initial_parse_selector_limit_error)?;
    estimate.selector_count = estimate
        .selector_count
        .checked_add(selector_count)
        .ok_or_else(initial_parse_selector_limit_error)?;
    estimate.selector_ast_bytes = estimate
        .selector_ast_bytes
        .checked_add(selector_headers)
        .and_then(|bytes| bytes.checked_add(selector_nodes))
        .ok_or_else(initial_parse_selector_limit_error)?;

    let declaration_vectors = storage_selector_count
        .checked_add(1)
        .ok_or_else(initial_parse_selector_limit_error)?;
    let suffix_declaration_storage = declaration_vectors
        .checked_mul(suffix_declaration_count)
        .and_then(|count| count.checked_mul(std::mem::size_of::<simplecss::Declaration<'static>>()))
        .and_then(|bytes| bytes.checked_mul(INITIAL_PARSE_DECLARATION_CAPACITY_FACTOR))
        .ok_or_else(initial_parse_selector_limit_error)?;
    let current_rule_prefix_storage = current_rule_selector_count
        .checked_add(1)
        .and_then(|vectors| vectors.checked_mul(current_rule_declaration_count))
        .and_then(|count| count.checked_mul(std::mem::size_of::<simplecss::Declaration<'static>>()))
        .and_then(|bytes| bytes.checked_mul(INITIAL_PARSE_DECLARATION_CAPACITY_FACTOR))
        .ok_or_else(initial_parse_selector_limit_error)?;
    let declaration_storage = suffix_declaration_storage
        .checked_add(current_rule_prefix_storage)
        .ok_or_else(initial_parse_selector_limit_error)?;
    estimate.declaration_storage_bytes = estimate
        .declaration_storage_bytes
        .checked_add(declaration_storage)
        .ok_or_else(initial_parse_selector_limit_error)?;
    Ok(estimate)
}

fn simplecss_recovery_delimiter_counts(
    bytes: &[u8],
    starts_in_selector_header: bool,
) -> Option<(usize, usize)> {
    let mut selector_delimiters = 0usize;
    let mut declaration_delimiters = 0usize;
    let mut cursor = 0usize;
    let mut function_depth = 0usize;
    let mut block_depth = usize::from(!starts_in_selector_header);
    let mut selector_header_pending = starts_in_selector_header;
    let mut current_body_start = None;

    while cursor < bytes.len() {
        if block_depth == 0
            && !selector_header_pending
            && bytes[cursor] == b'@'
            && is_simplecss_name_start(bytes.get(cursor + 1).copied())
        {
            cursor = skip_simplecss_at_rule(bytes, cursor);
            continue;
        }

        if bytes[cursor..].starts_with(b"/*") {
            cursor = simplecss_comment_end(bytes, cursor)?;
            continue;
        }

        match bytes[cursor] {
            b'\'' | b'"' => {
                cursor = simplecss_recovery_string_end(bytes, cursor)?;
            }
            b'(' => {
                function_depth = function_depth.checked_add(1)?;
                cursor += 1;
            }
            b')' => {
                function_depth = function_depth.saturating_sub(1);
                cursor += 1;
            }
            b',' if function_depth == 0 && block_depth == 0 => {
                selector_delimiters = selector_delimiters.checked_add(1)?;
                cursor += 1;
            }
            b'{' if function_depth == 0 => {
                if block_depth == 0 {
                    selector_delimiters = selector_delimiters.checked_add(1)?;
                    selector_header_pending = false;
                    current_body_start = Some(cursor + 1);
                } // cov:ignore: LLVM maps this block terminator without a counter.
                block_depth = block_depth.checked_add(1)?;
                cursor += 1;
            }
            b';' if function_depth == 0 && block_depth > 0 => {
                declaration_delimiters = declaration_delimiters.checked_add(1)?;
                cursor += 1;
            }
            b'}' if function_depth == 0 => {
                declaration_delimiters = declaration_delimiters.checked_add(1)?;
                block_depth = block_depth.saturating_sub(1);
                if block_depth == 0 {
                    if let Some(body_start) = current_body_start.take()
                        && simplecss_recovery_rule_body_requires_raw_recovery(bytes, body_start)?
                    {
                        return None;
                    }
                    selector_header_pending = false;
                } // cov:ignore: LLVM maps this block terminator without a counter.
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }

    if function_depth != 0 {
        return None;
    }

    Some((
        selector_delimiters.checked_add(1)?,
        declaration_delimiters.checked_add(1)?,
    ))
}

fn simplecss_raw_recovery_delimiter_counts(bytes: &[u8]) -> Option<(usize, usize)> {
    let selector_count = bytes
        .iter()
        .filter(|byte| matches!(byte, b',' | b'{'))
        .count()
        .checked_add(1)?;
    let declaration_count = bytes
        .iter()
        .filter(|byte| matches!(byte, b';' | b'}'))
        .count()
        .checked_add(1)?;
    Some((selector_count, declaration_count))
}

fn simplecss_recovery_rule_body_requires_raw_recovery(
    stylesheet: &[u8],
    body_start: usize,
) -> Option<bool> {
    let body_end = simplecss_recovery_rule_body_end(stylesheet, body_start)?;
    let body = std::str::from_utf8(stylesheet.get(body_start..body_end)?).ok()?;
    let lexical_declarations = simplecss_recovery_lexical_declaration_count(body.as_bytes())?;
    let (parsed_declarations, consumed_body) = simplecss_recovery_declaration_tokens(body)?;
    Some(!consumed_body || lexical_declarations != parsed_declarations)
}

fn simplecss_recovery_selector_header_requires_raw_recovery(
    stylesheet: &[u8],
    selector_start: usize,
) -> Option<bool> {
    let mut cursor = selector_start;
    let mut selector_segment_start = selector_start;
    let mut function_depth = 0usize;
    let mut requires_raw_recovery = false;

    while cursor < stylesheet.len() {
        if stylesheet[cursor..].starts_with(b"/*") {
            cursor = simplecss_comment_end(stylesheet, cursor)?;
            continue;
        }

        match stylesheet[cursor] {
            b'\'' | b'"' => cursor = simplecss_recovery_string_end(stylesheet, cursor)?,
            b'(' => {
                function_depth = function_depth.checked_add(1)?;
                cursor += 1;
            }
            b')' => {
                function_depth = function_depth.checked_sub(1)?;
                cursor += 1;
            }
            b',' if function_depth == 0 => {
                let segment = std::str::from_utf8(stylesheet.get(selector_segment_start..cursor)?)
                    .ok()?
                    .trim();
                requires_raw_recovery |= !simplecss_recovery_selector_segment_is_valid(segment);
                selector_segment_start = cursor + 1;
                cursor += 1;
            }
            b'{' if function_depth == 0 => {
                let segment = std::str::from_utf8(stylesheet.get(selector_segment_start..cursor)?)
                    .ok()?
                    .trim();
                requires_raw_recovery |= !simplecss_recovery_selector_segment_is_valid(segment);
                return Some(requires_raw_recovery);
            }
            b'}' if function_depth == 0 => return None,
            _ => cursor += 1,
        }
    }

    None
}

fn simplecss_recovery_selector_segment_is_valid(selector: &str) -> bool {
    use simplecss::SelectorToken;

    let mut has_component = false;
    for token in simplecss::SelectorTokenizer::from(selector) {
        let Ok(token) = token else {
            return false;
        };

        if let SelectorToken::PseudoClass(name) = token
            && !matches!(
                name,
                "first-child" | "link" | "visited" | "hover" | "active" | "focus"
            )
        {
            return false;
        }

        if !matches!(
            token,
            SelectorToken::DescendantCombinator
                | SelectorToken::ChildCombinator
                | SelectorToken::AdjacentCombinator
        ) {
            has_component = true;
        }
    }

    has_component
}

fn simplecss_recovery_rule_body_end(stylesheet: &[u8], body_start: usize) -> Option<usize> {
    let mut cursor = body_start;
    let mut function_depth = 0usize;
    let mut nested_blocks = 0usize;

    while cursor < stylesheet.len() {
        if stylesheet[cursor..].starts_with(b"/*") {
            cursor = simplecss_comment_end(stylesheet, cursor)?;
            continue;
        }

        match stylesheet[cursor] {
            b'\'' | b'"' => cursor = simplecss_recovery_string_end(stylesheet, cursor)?,
            b'(' => {
                function_depth = function_depth.checked_add(1)?;
                cursor += 1;
            }
            b')' => {
                function_depth = function_depth.checked_sub(1)?;
                cursor += 1;
            }
            b'{' if function_depth == 0 => {
                nested_blocks = nested_blocks.checked_add(1)?;
                cursor += 1;
            }
            b'}' if function_depth == 0 && nested_blocks == 0 => return Some(cursor),
            b'}' if function_depth == 0 => {
                nested_blocks -= 1;
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }

    None
}

fn simplecss_recovery_lexical_declaration_count(bytes: &[u8]) -> Option<usize> {
    let mut cursor = 0usize;
    let mut function_depth = 0usize;
    let mut declaration_count = 0usize;
    let mut current_declaration_has_content = false;

    while cursor < bytes.len() {
        if bytes[cursor..].starts_with(b"/*") {
            cursor = simplecss_comment_end(bytes, cursor)?;
            continue;
        }

        match bytes[cursor] {
            b'\'' | b'"' => {
                cursor = simplecss_recovery_string_end(bytes, cursor)?;
                current_declaration_has_content = true;
            }
            b'(' => {
                function_depth = function_depth.checked_add(1)?;
                current_declaration_has_content = true;
                cursor += 1;
            }
            b')' => {
                function_depth = function_depth.checked_sub(1)?;
                cursor += 1;
            }
            b';' if function_depth == 0 => {
                if current_declaration_has_content {
                    declaration_count = declaration_count.checked_add(1)?;
                } // cov:ignore: LLVM maps this block terminator without a counter.
                current_declaration_has_content = false;
                cursor += 1;
            }
            byte => {
                if !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0C') {
                    current_declaration_has_content = true;
                }
                cursor += 1;
            }
        }
    }

    if current_declaration_has_content {
        declaration_count = declaration_count.checked_add(1)?;
    }
    Some(declaration_count)
}

fn simplecss_recovery_declaration_tokens(body: &str) -> Option<(usize, bool)> {
    let bytes = body.as_bytes();
    let mut cursor = 0usize;
    let mut declaration_count = 0usize;

    for declaration in simplecss::DeclarationTokenizer::from(body) {
        let name_start = simplecss_recovery_source_offset(body, declaration.name)?;
        if !skip_simplecss_spaces_and_comments(bytes, &mut cursor) || cursor != name_start {
            return Some((declaration_count, false)); // cov:ignore: Tokenizer enforces this before yielding.
        }
        cursor = simplecss_recovery_source_offset(body, declaration.value)?
            .checked_add(declaration.value.len())?;
        if !skip_simplecss_spaces_and_comments(bytes, &mut cursor) {
            return Some((declaration_count, false)); // cov:ignore: Tokenizer enforces this before yielding.
        }

        if declaration.important {
            if bytes.get(cursor) != Some(&b'!') {
                return Some((declaration_count, false)); // cov:ignore: Tokenizer enforces its important marker.
            }
            cursor += 1;
            if !skip_simplecss_spaces_and_comments(bytes, &mut cursor)
                || !bytes[cursor..].starts_with(b"important")
            {
                return Some((declaration_count, false)); // cov:ignore: Tokenizer enforces its important marker.
            }
            cursor += b"important".len();
        } else if bytes.get(cursor) == Some(&b'!') {
            return Some((declaration_count, false));
        }

        if !skip_simplecss_spaces_and_comments(bytes, &mut cursor) {
            return Some((declaration_count, false)); // cov:ignore: Tokenizer enforces this before yielding.
        }
        while bytes.get(cursor) == Some(&b';') {
            cursor += 1;
            if !skip_simplecss_spaces_and_comments(bytes, &mut cursor) {
                return Some((declaration_count, false)); // cov:ignore: Tokenizer enforces this before yielding.
            }
        }

        declaration_count = declaration_count.checked_add(1)?;
    }

    if !skip_simplecss_spaces_and_comments(bytes, &mut cursor) {
        return Some((declaration_count, false));
    }
    Some((declaration_count, cursor == bytes.len()))
}

fn simplecss_recovery_source_offset(source: &str, slice: &str) -> Option<usize> {
    let start = (slice.as_ptr() as usize).checked_sub(source.as_ptr() as usize)?;
    let end = start.checked_add(slice.len())?;
    (source.get(start..end)? == slice).then_some(start)
}

fn simplecss_recovery_string_end(bytes: &[u8], cursor: usize) -> Option<usize> {
    let quote = *bytes.get(cursor)?;
    let mut previous = quote;
    let mut end = cursor.checked_add(1)?;
    while let Some(byte) = bytes.get(end).copied() {
        if byte == quote && previous != b'\\' {
            return end.checked_add(1);
        }
        previous = byte;
        end += 1;
    }
    None
}

fn simplecss_has_block_delimiter(bytes: &[u8]) -> bool {
    bytes.iter().any(|byte| matches!(byte, b'{' | b'}'))
}

fn skip_simplecss_spaces_and_comments(bytes: &[u8], cursor: &mut usize) -> bool {
    loop {
        while bytes
            .get(*cursor)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0C'))
        {
            *cursor += 1;
        }
        if !bytes[*cursor..].starts_with(b"/*") {
            return true;
        }
        let Some(comment_end) = simplecss_comment_end(bytes, *cursor) else {
            *cursor = bytes.len();
            return false;
        };
        *cursor = comment_end;
    }
}

fn simplecss_comment_end(bytes: &[u8], cursor: usize) -> Option<usize> {
    let end = bytes
        .get(cursor + 2..)?
        .windows(2)
        .position(|pair| pair == b"*/")?;
    Some(cursor + 2 + end + 2)
}

fn simplecss_string_end(bytes: &[u8], cursor: usize) -> usize {
    let quote = bytes[cursor];
    let mut previous = quote;
    let mut end = cursor + 1;
    while let Some(byte) = bytes.get(end).copied() {
        if byte == quote && previous != b'\\' {
            return end + 1;
        }
        previous = byte;
        end += 1;
    }
    bytes.len()
}

fn simplecss_function_end(bytes: &[u8], cursor: usize) -> usize {
    bytes[cursor + 1..]
        .iter()
        .position(|byte| *byte == b')')
        .map_or(bytes.len(), |offset| cursor + offset + 2)
}

fn is_simplecss_name_start(byte: Option<u8>) -> bool {
    byte.is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_' || byte > 237)
}

fn skip_simplecss_at_rule(bytes: &[u8], cursor: usize) -> usize {
    let mut end = cursor + 1;
    while end < bytes.len() && !matches!(bytes[end], b';' | b'{') {
        end += 1;
    }
    match bytes.get(end) {
        Some(b';') => end + 1,
        Some(b'{') => {
            end += 1;
            let mut nested_blocks = 0usize;
            while end < bytes.len() {
                match bytes[end] {
                    b'{' => nested_blocks += 1,
                    b'}' if nested_blocks == 0 => return end + 1,
                    b'}' => nested_blocks -= 1,
                    _ => {}
                }
                end += 1;
            }
            end
        }
        _ => end,
    }
}

fn svg_style_has_css_type(node: roxmltree::Node<'_, '_>) -> bool {
    node.attribute("type").is_none_or(|mime| {
        mime.trim().is_empty()
            || mime
                .split(';')
                .next()
                .is_some_and(|essence| essence.trim().eq_ignore_ascii_case("text/css"))
    })
}

fn normalize_svg_css_types(
    source: &str,
    budget: &mut SelectorFreezeBudget,
) -> Result<Option<String>, SvgError> {
    // Freeze selectors first so matching the original type attribute still works.
    budget.bytes(source.len())?;
    let xml = roxmltree::Document::parse(source)
        .map_err(|error| SvgError::InvalidDocument(error.to_string()))?;
    let mut edits = Vec::new();
    for node in xml.descendants().filter(|node| node.has_tag_name("style")) {
        if svg_style_has_css_type(node)
            && let Some(attribute) = node.attribute_node("type")
            && attribute.value() != "text/css"
        {
            budget.bytes(8 + std::mem::size_of::<(Range<usize>, String)>())?;
            edits.push((attribute.range_value(), "text/css".to_owned()));
        }
    }
    if edits.is_empty() {
        return Ok(None);
    }
    apply_selector_edits(source, edits, budget).map(Some)
}

fn preflight_initial_svg_selectors(xml: &roxmltree::Document<'_>) -> Result<(), SvgError> {
    let mut budget = InitialParseSelectorBudget::new();
    let mut selectors = Vec::new();
    let mut total_rules = 0usize;
    for node in xml.descendants().filter(|node| node.has_tag_name("style")) {
        if !svg_style_has_css_type(node) {
            continue;
        }
        let Some(stylesheet_text) = node.text() else {
            continue;
        };
        budget.charge_bytes(stylesheet_text.len())?;
        let estimate = estimate_initial_stylesheet_resources(stylesheet_text)?;
        budget.check_selector_capacity(estimate.selector_count)?;
        budget.charge_selector_ast_bytes(estimate.selector_ast_bytes)?;
        budget.charge_declaration_storage(estimate.declaration_storage_bytes)?;
        let stylesheet = simplecss::StyleSheet::parse(stylesheet_text);
        budget.charge_selectors(stylesheet.rules.len())?;
        total_rules = total_rules
            .checked_add(stylesheet.rules.len())
            .ok_or_else(initial_parse_selector_limit_error)?;
        budget.charge_stylesheet_sort_work(total_rules)?;
        selectors.extend(stylesheet.rules.into_iter().map(|rule| rule.selector));
    }
    if selectors.is_empty() {
        return Ok(());
    }

    let mut xml_node_count = 0usize;
    for _ in xml.descendants() {
        charge_initial_parse_node_count(&mut xml_node_count, MAX_INITIAL_PARSE_XML_NODES)?;
    }

    let mut id_map = HashMap::new();
    for node in xml.descendants() {
        budget.charge_work(1)?;
        if let Some(id) = node.attribute("id") {
            budget.charge_work(id.len())?;
            if !id_map.contains_key(id) {
                let id_storage_bytes = id
                    .len()
                    .checked_add(std::mem::size_of::<String>() * 2)
                    .ok_or_else(initial_parse_selector_limit_error)?;
                budget.charge_bytes(id_storage_bytes)?;
                id_map.insert(id.to_owned(), node);
            }
        }
    }

    let mut expanded_node_count = 0usize;
    preflight_svg_selector_children(
        xml.root(),
        xml.root(),
        1,
        &selectors,
        &id_map,
        &mut budget,
        &mut expanded_node_count,
    )
}

fn ensure_initial_svg_xml_depth_bounded(source: &str) -> Result<(), SvgError> {
    let bytes = source.as_bytes();
    let mut cursor = 0usize;
    let mut depth = 0usize;

    while cursor < bytes.len() {
        let Some(relative_start) = bytes[cursor..].iter().position(|byte| *byte == b'<') else {
            break;
        };
        cursor += relative_start;

        if bytes[cursor..].starts_with(b"<!--") {
            cursor = skip_xml_markup_until(bytes, cursor + 4, b"-->");
            continue;
        }
        if bytes[cursor..].starts_with(b"<![CDATA[") {
            cursor = skip_xml_markup_until(bytes, cursor + 9, b"]]>");
            continue;
        }
        if bytes[cursor..].starts_with(b"<?") {
            cursor = skip_xml_markup_until(bytes, cursor + 2, b"?>");
            continue;
        }

        let closing = bytes.get(cursor + 1) == Some(&b'/');
        let name_start = cursor + usize::from(closing) + 1;
        if !bytes.get(name_start).is_some_and(|byte| {
            byte.is_ascii_alphabetic() || matches!(byte, b'_' | b':') || *byte >= 0x80
        }) {
            cursor += 1;
            continue;
        }

        let mut quote = None;
        let mut end = name_start;
        while let Some(byte) = bytes.get(end).copied() {
            if let Some(quote_byte) = quote {
                if byte == quote_byte {
                    quote = None;
                }
            } else {
                match byte {
                    b'\'' | b'"' => quote = Some(byte),
                    b'>' => break,
                    _ => {}
                }
            }
            end += 1;
        }
        if end == bytes.len() {
            break;
        }

        if closing {
            depth = depth.saturating_sub(1);
        } else {
            let last_non_space = bytes[name_start..end]
                .iter()
                .rev()
                .find(|byte| !byte.is_ascii_whitespace());
            if last_non_space != Some(&b'/') {
                depth = depth
                    .checked_add(1)
                    .ok_or_else(initial_parse_selector_limit_error)?;
                if depth > MAX_INITIAL_PARSE_XML_DEPTH {
                    return Err(initial_parse_selector_limit_error());
                }
            }
        }

        cursor = end + 1;
    }

    Ok(())
}

fn skip_xml_markup_until(bytes: &[u8], start: usize, terminator: &[u8]) -> usize {
    bytes[start..]
        .windows(terminator.len())
        .position(|window| window == terminator)
        .map_or(bytes.len(), |offset| start + offset + terminator.len())
}

fn charge_initial_parse_node_count(count: &mut usize, limit: usize) -> Result<(), SvgError> {
    *count = count
        .checked_add(1)
        .ok_or_else(initial_parse_selector_limit_error)?;
    if *count > limit {
        return Err(initial_parse_selector_limit_error());
    }
    Ok(())
}

enum InitialSvgSelectorFrame<'a, 'input: 'a> {
    Visit {
        node: roxmltree::Node<'a, 'input>,
        origin: roxmltree::Node<'a, 'input>,
        depth: usize,
    },
    Children {
        origin: roxmltree::Node<'a, 'input>,
        depth: usize,
        children: roxmltree::Children<'a, 'input>,
    },
}

fn preflight_svg_selector_children<'a, 'input: 'a, 'css>(
    parent: roxmltree::Node<'a, 'input>,
    origin: roxmltree::Node<'a, 'input>,
    depth: usize,
    selectors: &[simplecss::Selector<'css>],
    id_map: &HashMap<String, roxmltree::Node<'a, 'input>>,
    budget: &mut InitialParseSelectorBudget,
    expanded_node_count: &mut usize,
) -> Result<(), SvgError> {
    let mut pending = vec![InitialSvgSelectorFrame::Children {
        origin,
        depth,
        children: parent.children(),
    }];

    while let Some(frame) = pending.pop() {
        let (node, origin, depth) = match frame {
            InitialSvgSelectorFrame::Children {
                origin,
                depth,
                mut children,
            } => {
                let Some(node) = children.next() else {
                    continue;
                };
                pending.push(InitialSvgSelectorFrame::Children {
                    origin,
                    depth,
                    children,
                });
                pending.push(InitialSvgSelectorFrame::Visit {
                    node,
                    origin,
                    depth,
                });
                continue;
            }
            InitialSvgSelectorFrame::Visit {
                node,
                origin,
                depth,
            } => (node, origin, depth),
        };

        if !node.is_element()
            || node
                .tag_name()
                .namespace()
                .is_some_and(|namespace| namespace != SVG_NAMESPACE)
        {
            continue;
        }

        let tag_name = node.tag_name().name();
        if tag_name == "style" {
            continue;
        }
        if depth > MAX_INITIAL_PARSE_XML_DEPTH {
            return Err(initial_parse_selector_limit_error());
        }

        charge_initial_parse_node_count(expanded_node_count, MAX_INITIAL_PARSE_XML_NODES)?;

        for selector in selectors {
            budget.charge_work(1)?;
            budgeted_selector_matches_with_budget(
                selector,
                node,
                &mut budget.remaining_work,
                initial_parse_selector_limit_error,
            )?;
        }

        if tag_name == "use" {
            let Some(link) = resolve_svg_use_reference(node, id_map) else {
                continue;
            };
            if link == node || link == origin {
                continue;
            }
            if svg_use_expansion_is_recursive(node, link, id_map, budget)? {
                continue;
            }
            pending.push(InitialSvgSelectorFrame::Visit {
                node: link,
                origin: node,
                depth: depth + 1,
            });
            continue;
        }

        pending.push(InitialSvgSelectorFrame::Children {
            origin,
            depth: depth + 1,
            children: node.children(),
        });
    }

    Ok(())
}

fn resolve_svg_use_reference<'a, 'input: 'a>(
    node: roxmltree::Node<'a, 'input>,
    id_map: &HashMap<String, roxmltree::Node<'a, 'input>>,
) -> Option<roxmltree::Node<'a, 'input>> {
    let link_value = node
        .attributes()
        .find(|attribute| attribute.name() == "href" && attribute.namespace().is_none())
        .or_else(|| {
            node.attributes().find(|attribute| {
                attribute.name() == "href" && attribute.namespace() == Some(XLINK_NAMESPACE)
            })
        })
        .map(|attribute| attribute.value())?;
    let link_id = svgtypes::IRI::from_str(link_value).ok()?.0;
    id_map.get(link_id).copied()
}

fn svg_use_expansion_is_recursive<'a, 'input: 'a>(
    use_node: roxmltree::Node<'a, 'input>,
    link: roxmltree::Node<'a, 'input>,
    id_map: &HashMap<String, roxmltree::Node<'a, 'input>>,
    budget: &mut InitialParseSelectorBudget,
) -> Result<bool, SvgError> {
    for link_child in link.descendants().skip(1) {
        budget.charge_work(1)?;
        if link_child.has_tag_name((SVG_NAMESPACE, "use"))
            && resolve_svg_use_reference(link_child, id_map)
                .is_some_and(|nested_link| nested_link == use_node || nested_link == link)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn without_svg11_public_doctype(source: &str) -> Result<std::borrow::Cow<'_, str>, SvgError> {
    const DECLARATION: &str = r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">"#;
    let Some(start) = doctype_declaration_start(source) else {
        return Ok(std::borrow::Cow::Borrowed(source));
    };
    if !source[start..].starts_with(DECLARATION) {
        return Err(SvgError::UnsupportedDoctype);
    }
    let end = start + DECLARATION.len();
    if doctype_declaration_start(&source[end..]).is_some() {
        return Err(SvgError::UnsupportedDoctype);
    }
    let mut normalized = source[..start].to_owned();
    normalized.push_str(&source[end..]);
    Ok(std::borrow::Cow::Owned(normalized))
}

fn doctype_declaration_start(source: &str) -> Option<usize> {
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
            return Some(start);
        }
        cursor = start + 1;
    }
    None
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

fn reject_exported_external_references(source: &str) -> Result<(), SvgError> {
    let mut budget = SelectorFreezeBudget::new();
    budget.bytes(source.len())?;
    let xml = roxmltree::Document::parse(source)
        .map_err(|error| SvgError::InvalidDocument(error.to_string()))?;
    for node in xml.descendants() {
        if node.pi().is_some_and(|pi| pi.target == "xml-stylesheet") {
            return Err(SvgError::ExternalReference);
        }
        if !node.is_element() {
            continue;
        }
        let navigation_link = node.tag_name().name() == "a"
            && node
                .tag_name()
                .namespace()
                .is_none_or(|ns| ns == SVG_NAMESPACE);
        for attribute in node.attributes() {
            if matches!(attribute.name(), "href" | "src")
                && !(navigation_link && attribute.name() == "href")
                && !attribute.value().trim().starts_with('#')
            {
                return Err(SvgError::ExternalReference);
            }
            if attribute.namespace() == Some(XML_NAMESPACE)
                && attribute.name() == "base"
                && !attribute.value().trim().is_empty()
            {
                return Err(SvgError::ExternalReference);
            }
            if matches!(
                attribute.name(),
                "style"
                    | "fill"
                    | "stroke"
                    | "filter"
                    | "clip-path"
                    | "mask"
                    | "cursor"
                    | "marker"
                    | "marker-start"
                    | "marker-mid"
                    | "marker-end"
            ) {
                reject_external_css_references(attribute.value())?;
            }
        }
        if node.tag_name().name() == "style"
            && let Some(text) = node.text()
        {
            reject_external_css_references(text)?;
        }
    }
    Ok(())
}

fn reject_external_css_references(source: &str) -> Result<(), SvgError> {
    let mut input = cssparser::ParserInput::new(source);
    let mut parser = cssparser::Parser::new(&mut input);
    reject_external_css_tokens(&mut parser, 0).map_err(|error| match error.kind {
        cssparser::ParseErrorKind::Custom(error) => error,
        cssparser::ParseErrorKind::Basic(error) => {
            SvgError::InvalidDocument(format!("invalid SVG CSS resource reference: {error:?}"))
        }
    })
}

fn reject_external_css_tokens<'i>(
    input: &mut cssparser::Parser<'i, '_>,
    depth: usize,
) -> Result<(), cssparser::ParseError<'i, SvgError>> {
    if depth >= MAX_FILTER_CSS_NESTING {
        return Err(input.new_custom_error(SvgError::InvalidDocument(
            "SVG CSS resource reference nesting limit exceeded".into(),
        )));
    }
    while let Ok(token) = input.next().cloned() {
        match token {
            cssparser::Token::AtKeyword(name) if name.eq_ignore_ascii_case("import") => {
                return Err(input.new_custom_error(SvgError::ExternalReference));
            }
            cssparser::Token::UnquotedUrl(url) if !url.trim().starts_with('#') => {
                return Err(input.new_custom_error(SvgError::ExternalReference));
            }
            cssparser::Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                input.parse_nested_block(|nested| {
                    let url = nested.expect_string_cloned()?;
                    if !url.trim().starts_with('#') {
                        return Err(nested.new_custom_error(SvgError::ExternalReference));
                    }
                    Ok(())
                })?;
            }
            cssparser::Token::Function(_)
            | cssparser::Token::ParenthesisBlock
            | cssparser::Token::SquareBracketBlock
            | cssparser::Token::CurlyBracketBlock => {
                input.parse_nested_block(|nested| reject_external_css_tokens(nested, depth + 1))?;
            }
            _ => {}
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
        let is_style = node.tag_name().name() == "style" && svg_style_has_css_type(node);
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

fn viewport_matches_tree(tree: &usvg::Tree, width: f32, height: f32, has_view_box: bool) -> bool {
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

fn with_root_viewport_size(source: &str, width: f32, height: f32) -> Result<String, SvgError> {
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

fn normalize_svg_font_shorthands(
    source: &str,
    budget: &mut SelectorFreezeBudget,
) -> Result<String, SvgError> {
    budget.bytes(source.len())?;
    let xml = roxmltree::Document::parse(source)
        .map_err(|error| SvgError::InvalidDocument(error.to_string()))?;
    let mut edits = Vec::new();
    for node in xml
        .root_element()
        .descendants()
        .filter(|node| node.is_element())
    {
        if let Some(attribute) = node.attribute_node("style")
            && let Some(style) = expand_font_shorthands(attribute.value(), false, budget)?
        {
            let escaped = escape_xml_attribute(&style);
            edits.push((attribute.range(), format!("style=\"{escaped}\"")));
        }
        if node.tag_name().name() == "style" {
            for child in node.children().filter(|child| child.is_text()) {
                if let Some(style) =
                    expand_font_shorthands(child.text().unwrap_or(""), true, budget)?
                {
                    edits.push((child.range(), escape_xml_text(&style)));
                }
            }
        }
    }
    apply_selector_edits(source, edits, budget)
}

fn expand_font_shorthands(
    source: &str,
    stylesheet: bool,
    budget: &mut SelectorFreezeBudget,
) -> Result<Option<String>, SvgError> {
    budget.bytes(
        source
            .len()
            .checked_mul(8)
            .ok_or_else(selector_freeze_limit_error)?, // cov:ignore: The shared rewrite budget bounds source to 32 MiB, far below usize multiplication overflow.
    )?;
    let sheet;
    let declarations = if stylesheet {
        sheet = simplecss::StyleSheet::parse(source);
        sheet
            .rules
            .iter()
            .flat_map(|rule| rule.declarations.iter().copied())
            .collect::<Vec<_>>()
    } else {
        simplecss::DeclarationTokenizer::from(source).collect::<Vec<_>>()
    };
    let mut edits = Vec::new();
    for declaration in declarations
        .into_iter()
        .filter(|declaration| declaration.name.eq_ignore_ascii_case("font"))
    {
        let Ok(font) = svgtypes::FontShorthand::from_str(declaration.value) else {
            continue;
        };
        let offset = source_slice_offset(source, declaration.name);
        let exceeded = Cell::new(false);
        let mut input = cssparser::ParserInput::new(&source[offset..]);
        let mut parser = cssparser::Parser::new(&mut input);
        let mut declaration_parser = CssDeclarationSourceParser {
            source,
            budget,
            budget_exceeded: &exceeded,
        };
        let parsed = cssparser::RuleBodyParser::new(&mut parser, &mut declaration_parser)
            .next()
            .and_then(Result::ok)
            .ok_or_else(selector_freeze_limit_error)?;
        let important = if declaration.important {
            " !important"
        } else {
            ""
        };
        let properties = [
            ("font-style", font.font_style.unwrap_or("normal")),
            ("font-variant", font.font_variant.unwrap_or("normal")),
            ("font-weight", font.font_weight.unwrap_or("normal")),
            ("font-stretch", font.font_stretch.unwrap_or("normal")),
            ("line-height", "normal"),
            ("font-size-adjust", "none"),
            ("font-kerning", "auto"),
            ("font-variant-caps", "normal"),
            ("font-variant-ligatures", "normal"),
            ("font-variant-numeric", "normal"),
            ("font-variant-east-asian", "normal"),
            ("font-variant-position", "normal"),
            ("font-size", font.font_size),
            ("font-family", font.font_family),
        ];
        budget.bytes(
            declaration
                .value
                .len()
                .checked_add(1024)
                .ok_or_else(selector_freeze_limit_error)?, // cov:ignore: A declaration is a slice of budget-bounded source, so adding 1024 cannot overflow usize.
        )?;
        let replacement = properties
            .iter()
            .map(|(name, value)| format!("{name}:{value}{important}"))
            .collect::<Vec<_>>()
            .join(";");
        edits.push((parsed.range, replacement));
    }
    if edits.is_empty() {
        return Ok(None);
    }
    apply_selector_edits(source, edits, budget).map(Some)
}

fn valid_root_font(size: f32, family: &str) -> bool {
    if !size.is_finite() || size < 0.0 {
        return false;
    }
    let mut input = cssparser::ParserInput::new(family);
    let mut parser = cssparser::Parser::new(&mut input);
    let mut has_name = false;
    let mut quoted = false;
    while let Ok(token) = parser.next() {
        match token {
            cssparser::Token::Ident(_) if !quoted => has_name = true,
            cssparser::Token::QuotedString(_) if !has_name => {
                has_name = true;
                quoted = true;
            }
            cssparser::Token::Comma if has_name => {
                has_name = false;
                quoted = false;
            }
            _ => return false,
        }
    }
    has_name
}

fn with_root_style_overrides(
    source: &str,
    inherited_root_opacity: f32,
    neutralize_root_opacity: bool,
    host_controls_root_background: bool,
    source_style: SourceRootStyle<'_>,
    budget: &mut SelectorFreezeBudget,
) -> Result<String, SvgError> {
    let SourceRootStyle {
        color: root_color,
        font: root_font,
        font_face,
        visible,
        ..
    } = source_style;
    budget.bytes(source.len())?;
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
    if root_color.is_some() {
        root_style_properties.push("color");
        stylesheet_properties.push("color");
        if let Some(attribute) = root.attribute_node("color") {
            budget.bytes(2 * std::mem::size_of::<(Range<usize>, String)>())?;
            edits.push((attribute.range(), String::new()));
        }
    }
    if root_font.is_some() {
        root_style_properties.extend(["font-size", "font-family"]);
        stylesheet_properties.extend(["font-size", "font-family"]);
        for attribute in root
            .attributes()
            .filter(|attribute| matches!(attribute.name(), "font-size" | "font-family"))
        {
            budget.bytes(2 * std::mem::size_of::<(Range<usize>, String)>())?;
            edits.push((attribute.range(), String::new()));
        }
    }
    if font_face.is_some() {
        root_style_properties.extend(["font-weight", "font-style"]);
        stylesheet_properties.extend(["font-weight", "font-style"]);
    }
    if visible.is_some() {
        root_style_properties.push("visibility");
        stylesheet_properties.push("visibility");
    }
    for attribute in root.attributes().filter(|attribute| {
        (font_face.is_some() && matches!(attribute.name(), "font-weight" | "font-style"))
            || (visible.is_some() && attribute.name() == "visibility")
    }) {
        budget.bytes(2 * std::mem::size_of::<(Range<usize>, String)>())?;
        edits.push((attribute.range(), String::new()));
    }
    if host_controls_root_background {
        root_style_properties.extend(["background-color", "background"]);
        stylesheet_properties.extend(["background-color", "background"]);
        for attribute in root.attributes().filter(|attribute| {
            attribute.name() == "background-color" || attribute.name() == "background"
        }) {
            budget.bytes(2 * std::mem::size_of::<(Range<usize>, String)>())?;
            edits.push((attribute.range(), String::new()));
        }
    }

    if !root_style_properties.is_empty() {
        let existing_style = root.attribute("style");
        if let Some(style) = existing_style {
            budget.bytes(
                style
                    .len()
                    .checked_mul(8)
                    .ok_or_else(selector_freeze_limit_error)?,
            )?;
        }
        let retained = existing_style.map_or_else(String::new, |style| {
            strip_inline_style_properties(style, &root_style_properties)
        });
        let mut style_value = if neutralize_root_opacity {
            append_inline_declarations(&retained, &format!("opacity:{inherited_root_opacity}"))
        } else {
            retained
        };
        if let Some([red, green, blue, alpha]) = root_color {
            style_value = append_inline_declarations(
                &style_value,
                &format!(
                    "color:rgba({red},{green},{blue},{:.6})",
                    f32::from(alpha) / 255.0
                ),
            );
        }
        if let Some((size, family)) = root_font {
            budget.bytes(
                family
                    .len()
                    .checked_add(128)
                    .ok_or_else(selector_freeze_limit_error)?,
            )?;
            style_value = append_inline_declarations(
                &style_value,
                &format!("font-size:{size}px;font-family:{family}"),
            );
        }
        if let Some((weight, style)) = font_face {
            style_value = append_inline_declarations(
                &style_value,
                &format!(
                    "font-weight:{weight};font-style:{}",
                    style.to_ascii_lowercase()
                ),
            );
        }
        if let Some(visible) = visible {
            style_value = append_inline_declarations(
                &style_value,
                if visible {
                    "visibility:visible"
                } else {
                    "visibility:hidden"
                },
            );
        }
        let escaped_bytes = xml_attribute_escape_allocation_bytes(&style_value)?;
        budget.bytes(escaped_bytes)?;
        let escaped_style_value = escape_xml_attribute(&style_value);
        if let Some(attribute) = root
            .attributes()
            .find(|attribute| attribute.name() == "style")
        {
            let replacement_len = escaped_style_value
                .len()
                .checked_add(7)
                .ok_or_else(selector_freeze_limit_error)?;
            budget.bytes(
                replacement_len
                    .checked_add(2 * std::mem::size_of::<(Range<usize>, String)>())
                    .ok_or_else(selector_freeze_limit_error)?,
            )?;
            edits.push((
                attribute.range(),
                format!("style=\"{escaped_style_value}\""),
            ));
        } else if neutralize_root_opacity
            || root_color.is_some()
            || root_font.is_some()
            || visible.is_some()
        {
            let attribute_len = escaped_style_value
                .len()
                .checked_add(8)
                .ok_or_else(selector_freeze_limit_error)?;
            budget.bytes(attribute_len)?;
            inserted_root_attributes.push_str(" style=\"");
            inserted_root_attributes.push_str(&escaped_style_value);
            inserted_root_attributes.push('"');
        }
    }

    let scope_attribute = unique_scope_attribute(source, budget)?;
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
        let stylesheet_text_len = node
            .children()
            .filter(|child| child.is_text())
            .filter_map(|child| child.text())
            .try_fold(0usize, |length, text| {
                length
                    .checked_add(text.len())
                    .ok_or_else(selector_freeze_limit_error)
            })?;
        budget.bytes(
            stylesheet_text_len
                .checked_add(2 * std::mem::size_of::<String>())
                .ok_or_else(selector_freeze_limit_error)?,
        )?;
        let mut stylesheet_text = String::with_capacity(stylesheet_text_len);
        for text in node
            .children()
            .filter(|child| child.is_text())
            .filter_map(|child| child.text())
        {
            stylesheet_text.push_str(text);
        }
        if stylesheet_text.is_empty() {
            continue;
        }
        let Some(rewritten) = scope_stylesheet_properties(
            &stylesheet_text,
            &scope_attribute,
            &stylesheet_properties,
            budget,
        )?
        else {
            continue;
        };
        has_scoped_stylesheet_properties = true;
        let content_range = xml_element_content_range(source, node).ok_or_else(|| {
            SvgError::InvalidDocument("unterminated SVG style element".to_owned())
        })?;
        budget.bytes(xml_text_escape_allocation_bytes(&rewritten)?)?;
        let escaped = escape_xml_text(&rewritten);
        budget.bytes(2 * std::mem::size_of::<(Range<usize>, String)>())?;
        edits.push((content_range, escaped));
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
            SelectorFreezeBudget::consume(&mut budget.matches, 1)?;
            let insertion_text_len = scope_attribute
                .len()
                .checked_add(4)
                .ok_or_else(selector_freeze_limit_error)?;
            budget.bytes(
                insertion_text_len
                    .checked_add(2 * std::mem::size_of::<(Range<usize>, String)>())
                    .ok_or_else(selector_freeze_limit_error)?,
            )?;
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
        budget.bytes(2 * std::mem::size_of::<(Range<usize>, String)>())?;
        edits.push((insertion..insertion, inserted_root_attributes));
    }

    apply_selector_edits(source, edits, budget)
}

fn with_element_style_overrides(
    source: &str,
    elements: &[SvgElementStyle<'_>],
    budget: &mut SelectorFreezeBudget,
) -> Result<String, SvgError> {
    const PROPERTIES: &[&str] = &[
        "color",
        "display",
        "opacity",
        "visibility",
        "font-size",
        "font-family",
        "font-weight",
        "font-style",
    ];
    budget.bytes(source.len())?;
    let xml = roxmltree::Document::parse(source)
        .map_err(|error| SvgError::InvalidDocument(error.to_string()))?;
    let element_count = xml.descendants().filter(|node| node.is_element()).count();
    SelectorFreezeBudget::consume(&mut budget.checks, element_count)?;
    let mut overrides = BTreeMap::new();
    let mut scopes = BTreeMap::<String, (String, BTreeSet<usize>, bool)>::new();
    for element in elements {
        budget.bytes(
            element
                .declarations
                .len()
                .saturating_mul(8)
                .saturating_add(256),
        )?;
        if element.element_index == 0 || element.element_index >= element_count {
            return Err(SvgError::InvalidDocument(
                "invalid SVG descendant style index".into(),
            ));
        }
        let exceeded = Cell::new(false);
        let mut input = cssparser::ParserInput::new(element.declarations);
        let mut parser = cssparser::Parser::new(&mut input);
        let mut declaration_parser = CssDeclarationSourceParser {
            source: element.declarations,
            budget,
            budget_exceeded: &exceeded,
        };
        let mut keys = BTreeSet::new();
        for declaration in cssparser::RuleBodyParser::new(&mut parser, &mut declaration_parser) {
            let declaration = declaration.map_err(|_| {
                if exceeded.get() {
                    selector_freeze_limit_error()
                } else {
                    SvgError::InvalidDocument("invalid SVG descendant declaration".into())
                }
            })?;
            let Some(property) = PROPERTIES
                .iter()
                .find(|property| declaration.name.eq_ignore_ascii_case(property))
            else {
                return Err(SvgError::InvalidDocument(
                    "unsupported SVG descendant property".into(),
                ));
            };
            keys.insert((*property).to_owned());
        }
        if overrides
            .insert(element.element_index, (element, keys.clone()))
            .is_some()
        {
            return Err(SvgError::InvalidDocument(
                "duplicate SVG descendant style index".into(),
            ));
        }
        for key in keys {
            budget.bytes(key.len().saturating_add(256))?;
            let (_, overridden, _) = match scopes.entry(key) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    let name = unique_attribute_name(
                        source,
                        &format!("data-raikiri-svg-{}-scope", entry.key()),
                        budget,
                    )?;
                    entry.insert((name, BTreeSet::new(), false))
                }
            };
            overridden.insert(element.element_index);
        }
    }
    let mut edits = Vec::new();
    for node in xml
        .descendants()
        .filter(|node| node.has_tag_name("style") && svg_style_has_css_type(*node))
    {
        let Some(text) = node.text() else { continue };
        budget.bytes(text.len())?;
        let mut rewritten = text.to_owned();
        let mut changed = false;
        for (property, (scope, _, active)) in &mut scopes {
            if let Some(value) =
                scope_stylesheet_properties(&rewritten, scope, &[property.as_str()], budget)?
            {
                rewritten = value;
                *active = true;
                changed = true;
            }
        }
        if changed {
            let range = xml_element_content_range(source, node).ok_or_else(|| {
                SvgError::InvalidDocument("unterminated SVG style element".into())
            })?;
            budget.bytes(xml_text_escape_allocation_bytes(&rewritten)?)?;
            budget.bytes(2 * std::mem::size_of::<(Range<usize>, String)>())?;
            edits.push((range, escape_xml_text(&rewritten)));
        }
    }
    for (index, node) in xml
        .descendants()
        .filter(|node| node.is_element())
        .enumerate()
    {
        SelectorFreezeBudget::consume(&mut budget.checks, scopes.len().saturating_add(1))?;
        let mut inserted = String::new();
        if let Some((element, keys)) = overrides.get(&index) {
            let properties: Vec<_> = keys.iter().map(String::as_str).collect();
            budget.bytes(properties.len() * std::mem::size_of::<&str>())?;
            let existing = node.attribute("style").unwrap_or_default();
            budget.bytes(
                existing
                    .len()
                    .saturating_mul(8)
                    .saturating_add(element.declarations.len()),
            )?;
            let retained = strip_inline_style_properties(existing, &properties);
            let style = append_inline_declarations(&retained, element.declarations);
            budget.bytes(xml_attribute_escape_allocation_bytes(&style)?)?;
            let escaped = escape_xml_attribute(&style);
            for attribute in node.attributes().filter(|attribute| {
                attribute.namespace().is_none()
                    && keys
                        .iter()
                        .any(|key| attribute.name().eq_ignore_ascii_case(key))
            }) {
                budget.bytes(2 * std::mem::size_of::<(Range<usize>, String)>())?;
                edits.push((attribute.range(), String::new()));
            }
            if let Some(attribute) = node
                .attributes()
                .find(|attribute| attribute.namespace().is_none() && attribute.name() == "style")
            {
                budget.bytes(escaped.len().saturating_add(128))?;
                edits.push((attribute.range(), format!("style=\"{escaped}\"")));
            } else {
                budget.bytes(escaped.len().saturating_add(8))?;
                inserted.push_str(&format!(" style=\"{escaped}\""));
            }
        }
        for (scope, overridden, active) in scopes.values() {
            if *active && !overridden.contains(&index) {
                SelectorFreezeBudget::consume(&mut budget.matches, 1)?;
                budget.bytes(scope.len().saturating_add(8))?;
                inserted.push_str(&format!(" {scope}=\"\""));
            }
        }
        if !inserted.is_empty() {
            let end = root_start_tag_end(source, node.range().start)
                .ok_or_else(|| SvgError::InvalidDocument("unterminated SVG element tag".into()))?;
            let at = if end > node.range().start && source.as_bytes()[end - 1] == b'/' {
                end - 1
            } else {
                end
            };
            budget.bytes(2 * std::mem::size_of::<(Range<usize>, String)>())?;
            edits.push((at..at, inserted));
        }
    }
    apply_selector_edits(source, edits, budget)
}

struct OpacityDeclaration {
    value: String,
    important: bool,
    inline_style: bool,
    specificity: [u8; 3],
    source_order: usize,
}

// Intentionally stricter resource policy: raikiri-spike-plxjz. These bounds
// cover generated selector and root-style rewrite state, not the original
// backend CSS parser's allocations, and do not classify otherwise valid CSS
// as invalid grammar.
const MAX_SELECTOR_FREEZE_BYTES: usize = 32 * 1024 * 1024;
const MAX_SELECTOR_FREEZE_MATCHES: usize = 65_536;
const MAX_SELECTOR_FREEZE_CHECKS: usize = 1_048_576;
const MAX_SELECTOR_MATCH_STEPS: usize = 256;
const MAX_INITIAL_PARSE_SELECTOR_WORK: usize = 16 * 1024 * 1024;
const MAX_INITIAL_PARSE_SELECTOR_BYTES: usize = MAX_SELECTOR_FREEZE_BYTES;
const MAX_INITIAL_PARSE_SELECTORS: usize = 4_096;
// Bound SimpleCSS AST growth and its per-selector declaration-vector clones.
const MAX_INITIAL_PARSE_SELECTOR_AST_BYTES: usize = 32 * 1024 * 1024;
const MAX_INITIAL_PARSE_DECLARATION_STORAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_INITIAL_PARSE_SELECTOR_AST_BYTES_PER_SOURCE_BYTE: usize = 512;
const INITIAL_PARSE_DECLARATION_CAPACITY_FACTOR: usize = 2;
const MAX_INITIAL_PARSE_STYLE_SORT_WORK: usize = 4_194_304;
const MAX_INITIAL_PARSE_XML_NODES: usize = 1_000_000;
const MAX_INITIAL_PARSE_XML_DEPTH: usize = 128;

struct SelectorFreezeBudget {
    bytes: usize,
    matches: usize,
    checks: usize,
}

impl SelectorFreezeBudget {
    fn new() -> Self {
        Self {
            bytes: MAX_SELECTOR_FREEZE_BYTES,
            matches: MAX_SELECTOR_FREEZE_MATCHES,
            checks: MAX_SELECTOR_FREEZE_CHECKS,
        }
    }

    fn consume(remaining: &mut usize, amount: usize) -> Result<(), SvgError> {
        *remaining = remaining
            .checked_sub(amount)
            .ok_or_else(selector_freeze_limit_error)?;
        Ok(())
    }

    fn bytes(&mut self, amount: usize) -> Result<(), SvgError> {
        Self::consume(&mut self.bytes, amount)
    }
}

fn selector_freeze_limit_error() -> SvgError {
    SvgError::InvalidDocument("selector freezing resource limit exceeded".to_owned())
}

fn initial_parse_selector_limit_error() -> SvgError {
    SvgError::InvalidDocument("selector matching resource limit exceeded".to_owned())
}

struct InitialParseSelectorBudget {
    remaining_work: usize,
    remaining_bytes: usize,
    remaining_selectors: usize,
    remaining_selector_ast_bytes: usize,
    remaining_declaration_storage_bytes: usize,
    remaining_sort_work: usize,
}

impl InitialParseSelectorBudget {
    fn new() -> Self {
        Self {
            remaining_work: MAX_INITIAL_PARSE_SELECTOR_WORK,
            remaining_bytes: MAX_INITIAL_PARSE_SELECTOR_BYTES,
            remaining_selectors: MAX_INITIAL_PARSE_SELECTORS,
            remaining_selector_ast_bytes: MAX_INITIAL_PARSE_SELECTOR_AST_BYTES,
            remaining_declaration_storage_bytes: MAX_INITIAL_PARSE_DECLARATION_STORAGE_BYTES,
            remaining_sort_work: MAX_INITIAL_PARSE_STYLE_SORT_WORK,
        }
    }

    fn charge_work(&mut self, amount: usize) -> Result<(), SvgError> {
        self.remaining_work = self
            .remaining_work
            .checked_sub(amount)
            .ok_or_else(initial_parse_selector_limit_error)?;
        Ok(())
    }

    fn charge_bytes(&mut self, amount: usize) -> Result<(), SvgError> {
        self.remaining_bytes = self
            .remaining_bytes
            .checked_sub(amount)
            .ok_or_else(initial_parse_selector_limit_error)?;
        Ok(())
    }

    fn check_selector_capacity(&self, upper_bound: usize) -> Result<(), SvgError> {
        if upper_bound > self.remaining_selectors {
            return Err(initial_parse_selector_limit_error());
        }
        Ok(())
    }

    fn charge_selectors(&mut self, amount: usize) -> Result<(), SvgError> {
        self.remaining_selectors = self
            .remaining_selectors
            .checked_sub(amount)
            .ok_or_else(initial_parse_selector_limit_error)?;
        Ok(())
    }

    fn charge_selector_ast_bytes(&mut self, amount: usize) -> Result<(), SvgError> {
        self.remaining_selector_ast_bytes = self
            .remaining_selector_ast_bytes
            .checked_sub(amount)
            .ok_or_else(initial_parse_selector_limit_error)?;
        Ok(())
    }

    fn charge_declaration_storage(&mut self, amount: usize) -> Result<(), SvgError> {
        self.remaining_declaration_storage_bytes = self
            .remaining_declaration_storage_bytes
            .checked_sub(amount)
            .ok_or_else(initial_parse_selector_limit_error)?;
        Ok(())
    }

    fn charge_stylesheet_sort_work(&mut self, rule_count: usize) -> Result<(), SvgError> {
        let sort_passes = if rule_count <= 1 {
            usize::from(rule_count == 1)
        } else {
            (usize::BITS as usize) - (rule_count - 1).leading_zeros() as usize
        };
        let work = rule_count
            .checked_mul(sort_passes)
            .ok_or_else(initial_parse_selector_limit_error)?;
        self.remaining_sort_work = self
            .remaining_sort_work
            .checked_sub(work)
            .ok_or_else(initial_parse_selector_limit_error)?;
        Ok(())
    }
}

fn freeze_svg_stylesheet_selectors(
    source: &str,
    budget: &mut SelectorFreezeBudget,
) -> Result<String, SvgError> {
    budget.bytes(source.len())?;
    let xml = roxmltree::Document::parse(source)
        .map_err(|error| SvgError::InvalidDocument(error.to_string()))?;
    let root = xml.root_element();
    let style_nodes = xml
        .descendants()
        .filter(|node| node.has_tag_name("style") && svg_style_has_css_type(*node))
        .collect::<Vec<_>>();

    let mut stylesheet_ranges = style_nodes
        .iter()
        .filter_map(|node| xml_element_content_range(source, *node).map(|range| (*node, range)))
        .collect::<Vec<_>>();
    stylesheet_ranges.sort_by_key(|(_, range)| range.start);
    if stylesheet_ranges
        .windows(2)
        .any(|pair| pair[0].1.end > pair[1].1.start)
    {
        return Err(SvgError::InvalidDocument(
            "nested SVG style elements are unsupported".to_owned(),
        ));
    }

    let mut stylesheet = simplecss::StyleSheet::new();
    for node in &style_nodes {
        if let Some(text) = node.text() {
            stylesheet.parse_more(text);
        }
    }
    if stylesheet.rules.is_empty() {
        return Ok(source.to_owned());
    }

    let marker_prefix = unique_attribute_name(source, "data-raikiri-svg-selector", budget)?;
    let mut frozen_stylesheet = String::new();
    let mut element_markers = BTreeMap::<usize, Vec<String>>::new();
    for (index, rule) in stylesheet.rules.iter().enumerate() {
        // Charge a conservative suffix width before formatting the marker.
        budget.bytes(marker_prefix.len() + 1 + usize::BITS as usize)?;
        let marker = format!("{marker_prefix}-{index}");
        let mut has_match = false;
        for node in root.descendants().filter(|node| node.is_element()) {
            SelectorFreezeBudget::consume(&mut budget.checks, 1)?;
            // Markers cannot be inserted into source ranges that are replaced
            // wholesale as stylesheet text. Such nested markup is not a
            // renderable SVG element, even if a broad selector matches it.
            if stylesheet_ranges.iter().any(|(_, range)| {
                range.start <= node.range().start && node.range().start < range.end
            }) {
                continue;
            }
            if budgeted_selector_matches(&rule.selector, node, budget)? {
                SelectorFreezeBudget::consume(&mut budget.matches, 1)?;
                // Charge string storage plus a conservative per-match share
                // of vector/map/edit metadata before retaining the clone.
                budget.bytes(marker.len() + 128)?;
                has_match = true;
                element_markers
                    .entry(node.range().start)
                    .or_default()
                    .push(marker.clone());
            }
        }

        if !has_match {
            continue;
        }

        budget.bytes(marker.len() + 8)?;
        frozen_stylesheet.push('[');
        frozen_stylesheet.push_str(&marker);
        frozen_stylesheet.push_str("] {");
        for declaration in &rule.declarations {
            let declaration_bytes = declaration
                .name
                .len()
                .checked_add(declaration.value.len())
                .and_then(|len| len.checked_add(if declaration.important { 14 } else { 3 }))
                .ok_or_else(selector_freeze_limit_error)?;
            budget.bytes(declaration_bytes)?;
            frozen_stylesheet.push(' ');
            frozen_stylesheet.push_str(declaration.name);
            frozen_stylesheet.push(':');
            frozen_stylesheet.push_str(declaration.value);
            if declaration.important {
                frozen_stylesheet.push_str(" !important");
            }
            frozen_stylesheet.push(';');
        }
        frozen_stylesheet.push_str(" }\n");
    }

    let mut edits = Vec::<(Range<usize>, String)>::new();
    for (element_start, markers) in element_markers {
        let end = root_start_tag_end(source, element_start)
            .ok_or_else(|| SvgError::InvalidDocument("unterminated SVG element tag".to_owned()))?;
        let insertion = if end > element_start && source.as_bytes()[end - 1] == b'/' {
            end - 1
        } else {
            end
        };
        for marker in &markers {
            // Attribute formatting creates both a temporary and its copy.
            budget.bytes((marker.len() + 4) * 2)?;
        }
        let attributes = markers
            .iter()
            .map(|marker| format!(" {marker}=\"\""))
            .collect::<String>();
        edits.push((insertion..insertion, attributes));
    }

    let mut stylesheet_inserted = false;
    for (node, range) in stylesheet_ranges {
        let retained = retain_svg_at_rules(node.text().unwrap_or_default(), budget)?;
        let replacement = if !stylesheet_inserted {
            stylesheet_inserted = true;
            budget.bytes(retained.len())?;
            frozen_stylesheet.push_str(&retained);
            let escaped_len = frozen_stylesheet.bytes().try_fold(0usize, |len, byte| {
                len.checked_add(match byte {
                    b'&' => 5,
                    b'<' | b'>' => 4,
                    _ => 1,
                })
                .ok_or_else(selector_freeze_limit_error)
            })?;
            // escape_xml_text performs three replacement passes; charge their
            // maximum combined payload before any of them allocates.
            budget.bytes(
                escaped_len
                    .checked_mul(3)
                    .ok_or_else(selector_freeze_limit_error)?,
            )?;
            escape_xml_text(&frozen_stylesheet)
        } else {
            budget.bytes(xml_attribute_escape_allocation_bytes(&retained)?)?;
            escape_xml_text(&retained)
        };
        edits.push((range, replacement));
    }
    if !frozen_stylesheet.is_empty() && !stylesheet_inserted {
        return Err(SvgError::InvalidDocument(
            "could not rewrite SVG style element".to_owned(),
        ));
    }

    apply_selector_edits(source, edits, budget)
}

// SimpleCSS ignores at-rules. Keep their source for other vector consumers
// while freezing only the qualified rules that SimpleCSS actually matched.
fn retain_svg_at_rules(
    source: &str,
    budget: &mut SelectorFreezeBudget,
) -> Result<String, SvgError> {
    let mut input = cssparser::ParserInput::new(source);
    let mut parser = cssparser::Parser::new(&mut input);
    let mut retained = String::new();
    loop {
        let start = parser.position();
        let Ok(token) = parser.next_including_whitespace_and_comments() else {
            break;
        };
        SelectorFreezeBudget::consume(&mut budget.checks, 1)?;
        if !matches!(token, cssparser::Token::AtKeyword(_)) {
            continue;
        }
        loop {
            SelectorFreezeBudget::consume(&mut budget.checks, 1)?;
            match parser.next_including_whitespace_and_comments() {
                Ok(cssparser::Token::Semicolon) | Err(_) => break,
                Ok(cssparser::Token::CurlyBracketBlock) => {
                    let _ = parser.parse_nested_block(|body| {
                        while body.next().is_ok() {}
                        Ok::<(), cssparser::ParseError<'_, ()>>(())
                    });
                    break;
                }
                _ => {}
            }
        }
        let raw = parser.slice_from(start);
        budget.bytes(raw.len().saturating_add(1))?;
        retained.push_str(raw);
        retained.push('\n');
    }
    Ok(retained)
}

fn apply_selector_edits(
    source: &str,
    mut edits: Vec<(Range<usize>, String)>,
    budget: &mut SelectorFreezeBudget,
) -> Result<String, SvgError> {
    let final_len = edits.iter().try_fold(source.len(), |len, (range, text)| {
        len.checked_sub(range.len())
            .and_then(|len| len.checked_add(text.len()))
            .ok_or_else(selector_freeze_limit_error)
    })?;
    budget.bytes(final_len)?;
    edits.sort_by_key(|(range, _)| range.start);
    let mut result = String::with_capacity(final_len);
    let mut cursor = 0;
    for (range, replacement) in edits {
        if range.start < cursor {
            return Err(SvgError::InvalidDocument(
                "overlapping SVG selector edits".to_owned(),
            ));
        }
        result.push_str(&source[cursor..range.start]);
        result.push_str(&replacement);
        cursor = range.end;
    }
    result.push_str(&source[cursor..]);
    Ok(result)
}

fn normalize_svg_opacity_cascade(
    source: &str,
    budget: &mut SelectorFreezeBudget,
) -> Result<String, SvgError> {
    // Freeze winning opacity declarations before removing them from
    // stylesheets. Keep the literal `inherit` so referenced subtrees retain
    // their instance-specific inheritance through `<use>`.
    // Keep the freezing budget alive through this second generated form.
    // A long winning declaration must not be copied into every element for
    // free merely because the marker representation was small.
    budget.bytes(source.len())?;
    let xml = roxmltree::Document::parse(source)
        .map_err(|error| SvgError::InvalidDocument(error.to_string()))?;
    let root = xml.root_element();
    let mut stylesheets = Vec::new();
    for node in xml.descendants().filter(|node| node.has_tag_name("style")) {
        if !svg_style_has_css_type(node) {
            continue;
        }
        // Match usvg's stylesheet loader, which reads only `node.text()`.
        let Some(text) = node.text() else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let storage = text
            .len()
            .checked_add(2 * std::mem::size_of::<(roxmltree::Node<'_, '_>, String)>())
            .ok_or_else(selector_freeze_limit_error)?;
        budget.bytes(storage)?;
        stylesheets.push((node, text.to_owned()));
    }

    let mut stylesheet = simplecss::StyleSheet::new();
    for (_, text) in &stylesheets {
        stylesheet.parse_more(text);
    }
    let specificity_storage = stylesheet
        .rules
        .len()
        .checked_mul(std::mem::size_of::<[u8; 3]>())
        .ok_or_else(selector_freeze_limit_error)?;
    budget.bytes(specificity_storage)?;
    let mut specificities = Vec::with_capacity(stylesheet.rules.len());
    specificities.extend(
        stylesheet
            .rules
            .iter()
            .map(|rule| rule.selector.specificity()),
    );

    let mut edits = Vec::<(Range<usize>, String)>::new();
    for node in root.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().name() != "style"
            && node
                .tag_name()
                .namespace()
                .is_none_or(|namespace| namespace == SVG_NAMESPACE)
    }) {
        let winner = opacity_winner(node, &stylesheet, &specificities, budget)?;
        for attribute in node.attributes().filter(|attribute| {
            attribute.name() == "opacity"
                && matches!(
                    attribute.namespace(),
                    None | Some(SVG_NAMESPACE) | Some(XLINK_NAMESPACE) | Some(XML_NAMESPACE)
                )
        }) {
            edits.push((attribute.range(), String::new()));
        }

        if node == root {
            continue;
        }
        let Some(winner) = winner else {
            continue;
        };
        // `opacity` is not inherited by default, but an explicit `inherit`
        // must stay dynamic: usvg applies it in the `<use>` instance context.
        // Replacing it with the source element's computed number changes the
        // cloned element's opacity.
        let value = winner.value;
        let existing_style = node.attribute("style");
        // Bound retained declarations, intermediate formatting, all three
        // attribute-escape passes (up to 6x), and the completed edit payload.
        let generated_bound = existing_style
            .map_or(0, str::len)
            .checked_add(value.len())
            .and_then(|len| len.checked_add(10))
            .and_then(|len| len.checked_mul(32))
            .and_then(|len| len.checked_add(128))
            .ok_or_else(selector_freeze_limit_error)?;
        budget.bytes(generated_bound)?;
        let retained = existing_style.map_or_else(String::new, |style| {
            strip_inline_style_properties(style, &["opacity"])
        });
        let style_value = append_inline_declarations(&retained, &format!("opacity:{value}"));
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
        let Some(rewritten) = strip_stylesheet_properties(text, &["opacity"], budget)? else {
            continue;
        };
        let range = xml_element_content_range(source, *node).ok_or_else(|| {
            SvgError::InvalidDocument("unterminated SVG style element".to_owned())
        })?;
        budget.bytes(xml_text_escape_allocation_bytes(&rewritten)?)?;
        edits.push((range, escape_xml_text(&rewritten)));
    }

    apply_selector_edits(source, edits, budget)
}

fn opacity_winner(
    node: roxmltree::Node<'_, '_>,
    stylesheet: &simplecss::StyleSheet<'_>,
    specificities: &[[u8; 3]],
    budget: &mut SelectorFreezeBudget,
) -> Result<Option<OpacityDeclaration>, SvgError> {
    let stylesheet_work = stylesheet.rules.iter().try_fold(0usize, |work, rule| {
        work.checked_add(rule.declarations.len())
            .and_then(|work| work.checked_add(1))
            .ok_or_else(selector_freeze_limit_error)
    })?;
    SelectorFreezeBudget::consume(&mut budget.checks, stylesheet_work)?;
    let mut winner: Option<OpacityDeclaration> = None;

    let mut source_order = 0;
    for attribute in node.attributes().filter(|attribute| {
        attribute.name() == "opacity"
            && matches!(
                attribute.namespace(),
                None | Some(SVG_NAMESPACE) | Some(XLINK_NAMESPACE) | Some(XML_NAMESPACE)
            )
    }) {
        consider_opacity_declaration(
            &mut winner,
            attribute.value(),
            false,
            false,
            [0, 0, 0],
            source_order,
            budget,
        )?;
        source_order += 1;
    }

    // simplecss sorts by specificity and preserves source order for ties.
    for (rule, specificity) in stylesheet.rules.iter().zip(specificities) {
        let matches = budgeted_selector_matches(&rule.selector, node, budget)?;
        for declaration in &rule.declarations {
            if matches && declaration.name == "opacity" {
                consider_opacity_declaration(
                    &mut winner,
                    declaration.value,
                    declaration.important,
                    false,
                    *specificity,
                    source_order,
                    budget,
                )?;
            }
            source_order += 1;
        }
    }

    if let Some(style) = node.attribute("style") {
        for declaration in simplecss::DeclarationTokenizer::from(style) {
            if declaration.name == "opacity" {
                consider_opacity_declaration(
                    &mut winner,
                    declaration.value,
                    declaration.important,
                    true,
                    [0, 0, 0],
                    source_order,
                    budget,
                )?;
            }
            source_order += 1;
        }
    }

    Ok(winner)
}

fn consider_opacity_declaration(
    winner: &mut Option<OpacityDeclaration>,
    value: &str,
    important: bool,
    inline_style: bool,
    specificity: [u8; 3],
    source_order: usize,
    budget: &mut SelectorFreezeBudget,
) -> Result<(), SvgError> {
    budget.bytes(value.len() + std::mem::size_of::<OpacityDeclaration>())?;
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
        *winner = Some(candidate);
    }
    Ok(())
}

// Count Element callbacks and visited siblings inside simplecss's recursive matcher.
struct BudgetedSvgCssElement<'a, 'input: 'a, 'budget> {
    node: roxmltree::Node<'a, 'input>,
    remaining_checks: &'budget Cell<usize>,
    remaining_steps: &'budget Cell<usize>,
    exceeded: &'budget Cell<bool>,
}

impl<'a, 'input: 'a, 'budget> BudgetedSvgCssElement<'a, 'input, 'budget> {
    fn consume_step(&self) -> bool {
        let Some(remaining) = self.remaining_steps.get().checked_sub(1) else {
            self.exceeded.set(true);
            return false;
        };
        self.remaining_steps.set(remaining);
        self.consume_work(1)
    }

    fn consume_work(&self, amount: usize) -> bool {
        let Some(remaining) = self.remaining_checks.get().checked_sub(amount) else {
            self.exceeded.set(true);
            return false;
        };
        self.remaining_checks.set(remaining);
        true
    }

    fn wrap(&self, node: roxmltree::Node<'a, 'input>) -> Self {
        Self {
            node,
            remaining_checks: self.remaining_checks,
            remaining_steps: self.remaining_steps,
            exceeded: self.exceeded,
        }
    }
}

impl simplecss::Element for BudgetedSvgCssElement<'_, '_, '_> {
    fn parent_element(&self) -> Option<Self> {
        if !self.consume_step() {
            return None;
        }
        self.node.parent_element().map(|node| self.wrap(node))
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        if !self.consume_step() {
            return None;
        }
        let mut previous = self.node.prev_sibling();
        while let Some(node) = previous {
            if !self.consume_work(1) {
                return None;
            }
            if node.is_element() {
                return Some(self.wrap(node));
            }
            previous = node.prev_sibling();
        }
        None
    }

    fn has_local_name(&self, local_name: &str) -> bool {
        if !self.consume_step()
            || !self.consume_work(
                self.node
                    .tag_name()
                    .name()
                    .len()
                    .max(local_name.len())
                    .saturating_add(1),
            )
        {
            return false;
        }
        self.node.tag_name().name() == local_name
    }

    fn attribute_matches(
        &self,
        local_name: &str,
        operator: simplecss::AttributeOperator<'_>,
    ) -> bool {
        if !self.consume_step() {
            return false;
        }
        let mut value = None;
        for attribute in self.node.attributes() {
            if !self.consume_work(
                attribute
                    .name()
                    .len()
                    .max(local_name.len())
                    .saturating_add(1),
            ) {
                return false;
            }
            if attribute.name() == local_name {
                value = Some(attribute.value());
                break;
            }
        }
        let Some(value) = value else {
            return false;
        };
        if !self.consume_work(value.len().saturating_add(1)) {
            return false;
        }
        operator.matches(value)
    }

    fn pseudo_class_matches(&self, class: simplecss::PseudoClass<'_>) -> bool {
        if !self.consume_step() {
            return false;
        }
        match class {
            simplecss::PseudoClass::FirstChild => self.prev_sibling_element().is_none(),
            _ => false,
        }
    }
}

fn budgeted_selector_matches(
    selector: &simplecss::Selector<'_>,
    node: roxmltree::Node<'_, '_>,
    budget: &mut SelectorFreezeBudget,
) -> Result<bool, SvgError> {
    budgeted_selector_matches_with_budget(
        selector,
        node,
        &mut budget.checks,
        selector_freeze_limit_error,
    )
}

fn budgeted_selector_matches_with_budget(
    selector: &simplecss::Selector<'_>,
    node: roxmltree::Node<'_, '_>,
    available_work: &mut usize,
    limit_error: fn() -> SvgError,
) -> Result<bool, SvgError> {
    let remaining_checks = Cell::new(*available_work);
    let remaining_steps = Cell::new(MAX_SELECTOR_MATCH_STEPS);
    let exceeded = Cell::new(false);
    let element = BudgetedSvgCssElement {
        node,
        remaining_checks: &remaining_checks,
        remaining_steps: &remaining_steps,
        exceeded: &exceeded,
    };
    let matches = selector.matches(&element);
    *available_work = remaining_checks.get();
    if exceeded.get() {
        Err(limit_error())
    } else {
        Ok(matches)
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
    budget: &'a mut SelectorFreezeBudget,
    budget_exceeded: &'a Cell<bool>,
    removals: Vec<Range<usize>>,
    scoped_rules: Vec<String>,
}

struct CssDeclarationSourceParser<'a> {
    source: &'a str,
    budget: &'a mut SelectorFreezeBudget,
    budget_exceeded: &'a Cell<bool>,
}

fn consume_css_budget<'i, 't>(
    input: &mut cssparser::Parser<'i, 't>,
    budget: &mut SelectorFreezeBudget,
    amount: Option<usize>,
    budget_exceeded: &Cell<bool>,
) -> Result<(), cssparser::ParseError<'i, ()>> {
    if amount.is_some_and(|amount| budget.bytes(amount).is_ok()) {
        Ok(())
    } else {
        budget_exceeded.set(true);
        Err(input.new_custom_error(()))
    }
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
        let storage = name
            .len()
            .checked_add(raw.len())
            .and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<ParsedCssDeclaration>()));
        consume_css_budget(input, self.budget, storage, self.budget_exceeded)?;
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
        let selector_len = input
            .position()
            .byte_index()
            .saturating_sub(start.byte_index());
        let storage = selector_len.checked_add(2 * std::mem::size_of::<String>());
        consume_css_budget(input, self.budget, storage, self.budget_exceeded)?;
        Ok(input.slice(start..input.position()).trim().to_owned())
    }

    fn parse_block<'t>(
        &mut self,
        selector_list: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut cssparser::Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, cssparser::ParseError<'i, Self::Error>> {
        let declarations = {
            let mut declaration_parser = CssDeclarationSourceParser {
                source: self.source,
                budget: self.budget,
                budget_exceeded: self.budget_exceeded,
            };
            cssparser::RuleBodyParser::new(input, &mut declaration_parser)
                .filter_map(Result::ok)
                .collect::<Vec<_>>()
        };
        if self.budget_exceeded.get() {
            return Err(input.new_custom_error(()));
        }

        let scoped_declaration_count = declarations
            .iter()
            .filter(|declaration| {
                self.properties
                    .iter()
                    .any(|property| declaration.name.eq_ignore_ascii_case(property))
            })
            .count();
        if scoped_declaration_count == 0 {
            return Ok(());
        }

        let scoped_declaration_refs_bytes =
            scoped_declaration_count.checked_mul(2 * std::mem::size_of::<&ParsedCssDeclaration>());
        consume_css_budget(
            input,
            self.budget,
            scoped_declaration_refs_bytes,
            self.budget_exceeded,
        )?;
        let scoped_declarations = declarations
            .iter()
            .filter(|declaration| {
                self.properties
                    .iter()
                    .any(|property| declaration.name.eq_ignore_ascii_case(property))
            })
            .collect::<Vec<_>>();

        let removal_storage =
            scoped_declaration_count.checked_mul(2 * std::mem::size_of::<Range<usize>>());
        consume_css_budget(input, self.budget, removal_storage, self.budget_exceeded)?;

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
        let raw_storage = scoped_declaration_count
            .checked_mul(2 * std::mem::size_of::<String>())
            .and_then(|metadata| {
                scoped_declarations
                    .iter()
                    .try_fold(metadata, |bytes, declaration| {
                        bytes.checked_add(declaration.raw.len())
                    })
            });
        consume_css_budget(input, self.budget, raw_storage, self.budget_exceeded)?;
        let raw_declarations = scoped_declarations
            .iter()
            .map(|declaration| declaration.raw.clone())
            .collect::<Vec<_>>();
        if let Some(scope_attribute) = self.scope_attribute {
            let selector_count = selector_list
                .bytes()
                .filter(|byte| *byte == b',')
                .count()
                .checked_add(1);
            let selector_storage =
                selector_count.and_then(|count| count.checked_mul(2 * std::mem::size_of::<&str>()));
            consume_css_budget(input, self.budget, selector_storage, self.budget_exceeded)?;
            for selector in split_css_selector_list(&selector_list) {
                let selector = selector.trim();
                if selector.is_empty() {
                    continue;
                }
                if let Some(rule_len) =
                    scoped_property_rule_len(selector, scope_attribute, &raw_declarations)
                {
                    let storage = rule_len.checked_add(2 * std::mem::size_of::<String>());
                    consume_css_budget(input, self.budget, storage, self.budget_exceeded)?;
                    let rule = scoped_property_rule(
                        selector,
                        scope_attribute,
                        &raw_declarations,
                        rule_len,
                    );
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
    capacity: usize,
) -> String {
    let insertion = selector_scope_insertion(selector);
    let mut scoped_rule = String::with_capacity(capacity);
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
    scoped_rule
}

fn scoped_property_rule_len(
    selector: &str,
    scope_attribute: &str,
    declarations: &[String],
) -> Option<usize> {
    if selector_scope_insertion(selector) == 0 {
        return None;
    }
    declarations.iter().try_fold(
        selector
            .len()
            .checked_add(scope_attribute.len())?
            .checked_add(6)?,
        |length, declaration| length.checked_add(declaration.len())?.checked_add(2),
    )
}

fn scope_stylesheet_properties(
    source: &str,
    scope_attribute: &str,
    properties: &[&str],
    budget: &mut SelectorFreezeBudget,
) -> Result<Option<String>, SvgError> {
    rewrite_stylesheet_properties(source, Some(scope_attribute), properties, budget)
}

fn strip_stylesheet_properties(
    source: &str,
    properties: &[&str],
    budget: &mut SelectorFreezeBudget,
) -> Result<Option<String>, SvgError> {
    rewrite_stylesheet_properties(source, None, properties, budget)
}

fn rewrite_stylesheet_properties(
    source: &str,
    scope_attribute: Option<&str>,
    properties: &[&str],
    budget: &mut SelectorFreezeBudget,
) -> Result<Option<String>, SvgError> {
    let budget_exceeded = Cell::new(false);
    let mut parser_state = ScopedStylesheetParser {
        source,
        scope_attribute,
        properties,
        budget,
        budget_exceeded: &budget_exceeded,
        removals: Vec::new(),
        scoped_rules: Vec::new(),
    };
    {
        let mut input = cssparser::ParserInput::new(source);
        let mut parser = cssparser::Parser::new(&mut input);
        for _ in cssparser::StyleSheetParser::new(&mut parser, &mut parser_state) {
            if budget_exceeded.get() {
                break;
            }
        }
    }
    if budget_exceeded.get() {
        return Err(selector_freeze_limit_error());
    }
    if parser_state.removals.is_empty()
        || (scope_attribute.is_some() && parser_state.scoped_rules.is_empty())
    {
        return Ok(None);
    }

    let ScopedStylesheetParser {
        budget,
        removals,
        scoped_rules,
        ..
    } = parser_state;
    apply_css_rewrite(source, removals, scoped_rules, budget).map(Some)
}

fn apply_css_rewrite(
    source: &str,
    removals: Vec<Range<usize>>,
    scoped_rules: Vec<String>,
    budget: &mut SelectorFreezeBudget,
) -> Result<String, SvgError> {
    SelectorFreezeBudget::consume(&mut budget.checks, removals.len())?;
    let removed_len = removals.iter().try_fold(0usize, |total, range| {
        total
            .checked_add(range.len())
            .ok_or_else(selector_freeze_limit_error)
    })?;
    let appended_len = scoped_rules.iter().try_fold(0usize, |total, rule| {
        total
            .checked_add(rule.len())
            .and_then(|total| total.checked_add(1))
            .ok_or_else(selector_freeze_limit_error)
    })?;
    let final_len = source
        .len()
        .checked_sub(removed_len)
        .and_then(|length| length.checked_add(appended_len))
        .ok_or_else(selector_freeze_limit_error)?;
    budget.bytes(final_len)?;
    let mut rewritten = String::with_capacity(final_len);
    let mut cursor = 0;
    for range in removals {
        if range.start < cursor || range.end > source.len() {
            return Err(SvgError::InvalidDocument(
                "overlapping SVG CSS rewrite ranges".to_owned(),
            ));
        }
        rewritten.push_str(&source[cursor..range.start]);
        cursor = range.end;
    }
    rewritten.push_str(&source[cursor..]);
    for scoped_rule in scoped_rules {
        rewritten.push('\n');
        rewritten.push_str(&scoped_rule);
    }
    Ok(rewritten)
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

fn unique_scope_attribute(
    source: &str,
    budget: &mut SelectorFreezeBudget,
) -> Result<String, SvgError> {
    unique_attribute_name(source, "data-raikiri-root-opacity-scope", budget)
}

fn unique_attribute_name(
    source: &str,
    base: &str,
    budget: &mut SelectorFreezeBudget,
) -> Result<String, SvgError> {
    let mut found_base = false;
    let mut max_suffix = 0usize;
    for (start, _) in source.match_indices(base) {
        found_base = true;
        let tail = &source[start + base.len()..];
        let Some(digits) = tail.strip_prefix('-') else {
            continue;
        };
        let digits_len = digits.bytes().take_while(u8::is_ascii_digit).count();
        if digits_len == 0 {
            continue;
        }
        let suffix = digits[..digits_len]
            .parse::<usize>()
            .map_err(|_| selector_freeze_limit_error())?;
        max_suffix = max_suffix.max(suffix);
    }

    let name = if !found_base {
        budget.bytes(base.len())?;
        base.to_owned()
    } else {
        let next_suffix = max_suffix
            .checked_add(1)
            .ok_or_else(selector_freeze_limit_error)?;
        let name_len = base
            .len()
            .checked_add(1 + decimal_digits(next_suffix))
            .ok_or_else(selector_freeze_limit_error)?;
        budget.bytes(name_len)?;
        format!("{base}-{next_suffix}")
    };
    Ok(name)
}

fn decimal_digits(value: usize) -> usize {
    value.ilog10() as usize + 1
}

fn xml_attribute_escape_allocation_bytes(value: &str) -> Result<usize, SvgError> {
    let first_pass = value
        .len()
        .checked_add(
            value
                .bytes()
                .filter(|byte| *byte == b'&')
                .count()
                .checked_mul(4)
                .ok_or_else(selector_freeze_limit_error)?,
        )
        .ok_or_else(selector_freeze_limit_error)?;
    let second_pass = first_pass
        .checked_add(
            value
                .bytes()
                .filter(|byte| *byte == b'<')
                .count()
                .checked_mul(3)
                .ok_or_else(selector_freeze_limit_error)?,
        )
        .ok_or_else(selector_freeze_limit_error)?;
    let third_pass = second_pass
        .checked_add(
            value
                .bytes()
                .filter(|byte| *byte == b'"')
                .count()
                .checked_mul(5)
                .ok_or_else(selector_freeze_limit_error)?,
        )
        .ok_or_else(selector_freeze_limit_error)?;
    first_pass
        .checked_add(second_pass)
        .and_then(|bytes| bytes.checked_add(third_pass))
        .ok_or_else(selector_freeze_limit_error)
}

fn xml_text_escape_allocation_bytes(value: &str) -> Result<usize, SvgError> {
    let first_pass = value
        .len()
        .checked_add(
            value
                .bytes()
                .filter(|byte| *byte == b'&')
                .count()
                .checked_mul(4)
                .ok_or_else(selector_freeze_limit_error)?,
        )
        .ok_or_else(selector_freeze_limit_error)?;
    let second_pass = first_pass
        .checked_add(
            value
                .bytes()
                .filter(|byte| *byte == b'<')
                .count()
                .checked_mul(3)
                .ok_or_else(selector_freeze_limit_error)?,
        )
        .ok_or_else(selector_freeze_limit_error)?;
    let third_pass = second_pass
        .checked_add(
            value
                .bytes()
                .filter(|byte| *byte == b'>')
                .count()
                .checked_mul(3)
                .ok_or_else(selector_freeze_limit_error)?,
        )
        .ok_or_else(selector_freeze_limit_error)?;
    first_pass
        .checked_add(second_pass)
        .and_then(|bytes| bytes.checked_add(third_pass))
        .ok_or_else(selector_freeze_limit_error)
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
    viewport: SvgViewport,
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
        viewport.width / svg_size.width(),
        viewport.height / svg_size.height(),
    );
    resvg::render(tree, transform, &mut pixmap.as_mut());

    if let Some(root_opacity) = neutralized_root_opacity
        .filter(|opacity| *opacity < 1.0 && !within_four_ulps_of_one(*opacity))
    {
        // usvg exposes the parsed tree but not a setter for the source root
        // group's opacity. Remove that final premultiplied group multiplier so
        // the host paint layer can apply it once around SVG content and box
        // decorations together. The source root background is omitted before
        // rendering because usvg stores it outside the source root group.
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

fn within_four_ulps_of_one(value: f32) -> bool {
    value <= 1.0 && 1.0_f32.to_bits().saturating_sub(value.to_bits()) <= 4
}

#[cfg(test)]
mod tests;
