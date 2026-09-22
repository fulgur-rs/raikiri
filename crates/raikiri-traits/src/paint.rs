//! Owned, renderer-neutral paint payloads for page consumers.
//!
//! This module is intentionally separate from [`crate::PageFragment`] geometry
//! and [`crate::PageFragmentEvent`] link metadata. A paint payload describes
//! *what* a consumer may paint, while page geometry describes *where* layout
//! placed source nodes. The two streams may be correlated by `page_index`, but
//! one does not contain or imply the other.
//!
//! The prototype is deliberately small and owned:
//!
//! - operations use page-local physical CSS-pixel coordinates, so a consumer
//!   does not apply `PageFragment::content_box.x/y` to them;
//! - `PagePaintPayload::operations` is the paint order and must be consumed in
//!   vector order; `PagePaintPayload::push` assigns a monotonic order number;
//!   `PushClip`/`PopClip` scopes must be balanced within one payload;
//! - resource bytes live in an `Arc`-backed [`PaintResourceBundle`], and an
//!   operation refers to a resource only by the opaque [`PaintResourceId`];
//! - no operation borrows a DOM, cascade, layout tree, font context, image
//!   cache, or renderer scene. Cloning a payload keeps its resource bytes alive;
//!   a missing resource ID is a producer/validation error, never an implicit
//!   transparent paint.
//! - the first boundary covers solid fills, four-sided borders, shadows,
//!   positioned glyphs, encoded images, rectangular clips, and per-operation
//!   affine transforms. Gradients, arbitrary paths, blend/filter groups, text
//!   cluster metadata, and network-backed resource providers remain staged
//!   follow-ups rather than leaking renderer-specific types into this API. A
//!   producer must not silently approximate an unsupported CSS paint feature;
//!   it should omit it only with an explicit diagnostic or add a later neutral
//!   operation variant.
//!
//! This is an additive boundary prototype. The current `render_streaming`
//! implementation still emits geometry through [`crate::RenderSink`] and link
//! metadata through [`crate::PageEventObserver`]; it does not fabricate paint
//! operations. A future producer can add a paint-aware entry point or compose
//! [`crate::PagePaintSink`] without changing either existing contract.

use std::sync::Arc;

use crate::dom::NodeId;

/// A normalized RGBA color for a neutral paint operation.
///
/// Components are in the inclusive `0.0..=1.0` range by convention. The
/// producer owns conversion from CSS color values; this type has no dependency
/// on a CSS or renderer color representation.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PaintColor {
    /// Red component.
    pub red: f32,
    /// Green component.
    pub green: f32,
    /// Blue component.
    pub blue: f32,
    /// Alpha component.
    pub alpha: f32,
}

impl PaintColor {
    /// Construct a normalized RGBA color.
    pub const fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }
}

/// A page-local physical CSS-pixel rectangle in a paint payload.
///
/// Unlike [`crate::PageFragmentRect`], this rectangle is already in the
/// physical page coordinate system. It is not content-box-relative and must
/// not receive a second page inset.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PaintRect {
    /// Physical page-local x coordinate.
    pub x: f32,
    /// Physical page-local y coordinate.
    pub y: f32,
    /// Rectangle width.
    pub width: f32,
    /// Rectangle height.
    pub height: f32,
}

impl PaintRect {
    /// Construct a page-local paint rectangle.
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Four-sided CSS-pixel values used by a neutral border operation.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PaintInsets {
    /// Top value.
    pub top: f32,
    /// Right value.
    pub right: f32,
    /// Bottom value.
    pub bottom: f32,
    /// Left value.
    pub left: f32,
}

impl PaintInsets {
    /// Construct top/right/bottom/left values.
    pub const fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }
}

/// A six-component affine transform in page-local CSS-pixel space.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PaintTransform {
    /// First row, first column.
    pub a: f32,
    /// Second row, first column.
    pub b: f32,
    /// First row, second column.
    pub c: f32,
    /// Second row, second column.
    pub d: f32,
    /// Translation x.
    pub e: f32,
    /// Translation y.
    pub f: f32,
}

impl PaintTransform {
    /// The identity transform.
    pub const fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Construct an affine transform.
    pub const fn new(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Self {
        Self { a, b, c, d, e, f }
    }
}

impl Default for PaintTransform {
    fn default() -> Self {
        Self::identity()
    }
}

/// Opaque identity for a font or image resource in one paint payload.
///
/// IDs are producer-assigned and meaningful only within the resource bundle
/// carried by the payload. Consumers must not interpret them as URLs, DOM
/// nodes, file descriptors, or renderer handles.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct PaintResourceId(pub u64);

impl PaintResourceId {
    /// Construct an opaque resource identity.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

/// Neutral resource data retained by a paint payload.
///
/// The prototype carries encoded bytes, not decoder- or renderer-specific
/// objects. Resource decoding and cache policy remain consumer-owned.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PaintResourceKind {
    /// Encoded image bytes such as PNG, JPEG, or SVG.
    Image {
        /// Media type supplied by the producer.
        media_type: String,
        /// Owned encoded bytes shared by cloned payloads.
        bytes: Arc<[u8]>,
    },
    /// Font bytes used by a glyph run.
    Font {
        /// Optional producer-resolved family label.
        family: Option<String>,
        /// Face index inside a collection font file.
        face_index: u32,
        /// Owned font bytes shared by cloned payloads.
        bytes: Arc<[u8]>,
    },
}

/// One resource in a neutral paint resource bundle.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PaintResource {
    /// Opaque identity referenced by paint operations.
    pub id: PaintResourceId,
    /// Resource kind and owned data.
    pub kind: PaintResourceKind,
}

impl PaintResource {
    /// Construct an encoded image resource.
    pub fn image(
        id: PaintResourceId,
        media_type: impl Into<String>,
        bytes: impl Into<Arc<[u8]>>,
    ) -> Self {
        Self {
            id,
            kind: PaintResourceKind::Image {
                media_type: media_type.into(),
                bytes: bytes.into(),
            },
        }
    }

    /// Construct a font resource for glyph runs.
    pub fn font(
        id: PaintResourceId,
        family: Option<String>,
        face_index: u32,
        bytes: impl Into<Arc<[u8]>>,
    ) -> Self {
        Self {
            id,
            kind: PaintResourceKind::Font {
                family,
                face_index,
                bytes: bytes.into(),
            },
        }
    }
}

/// Owned resources shared by one or more page paint payloads.
///
/// Resource IDs must be unique within a bundle. The prototype leaves
/// deduplication and decoding policy to the producer/consumer; it only makes
/// the ownership and lifetime boundary explicit.
#[derive(Debug, Default, Clone, PartialEq)]
#[non_exhaustive]
pub struct PaintResourceBundle {
    /// Resources indexed by their opaque IDs.
    pub resources: Vec<PaintResource>,
}

impl PaintResourceBundle {
    /// Construct an empty resource bundle.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one owned resource to the bundle.
    pub fn push(&mut self, resource: PaintResource) {
        self.resources.push(resource);
    }
}

/// Glyph placement data for one pre-shaped run.
///
/// The glyph ID is meaningful only with the referenced font resource. Text
/// shaping, Unicode segmentation, and fallback selection stay producer-owned;
/// a consumer receives positioned glyphs rather than Parley or fontique types.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PaintGlyph {
    /// Font-specific glyph identifier.
    pub glyph_id: u32,
    /// Page-local x position.
    pub x: f32,
    /// Page-local y position.
    pub y: f32,
    /// Advance in x.
    pub advance_x: f32,
    /// Advance in y.
    pub advance_y: f32,
}

impl PaintGlyph {
    /// Construct one positioned glyph.
    pub const fn new(glyph_id: u32, x: f32, y: f32, advance_x: f32, advance_y: f32) -> Self {
        Self {
            glyph_id,
            x,
            y,
            advance_x,
            advance_y,
        }
    }
}

/// A pre-shaped glyph run operation.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PaintGlyphRun {
    /// Font resource used to interpret `glyphs`.
    pub font: PaintResourceId,
    /// Used font size in CSS pixels.
    pub font_size: f32,
    /// Glyph brush color.
    pub color: PaintColor,
    /// Positioned glyphs in run order.
    pub glyphs: Vec<PaintGlyph>,
}

/// A solid fill operation, typically a background or canvas fill.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PaintFill {
    /// Fill color and alpha.
    pub color: PaintColor,
}

/// Border style values that do not depend on a CSS or renderer enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PaintBorderStyle {
    /// Continuous stroke.
    Solid,
    /// Dashed stroke.
    Dashed,
    /// Dotted stroke.
    Dotted,
    /// Double stroke.
    Double,
}

/// A four-sided border operation.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PaintBorder {
    /// Used border widths in CSS pixels.
    pub widths: PaintInsets,
    /// Border color.
    pub color: PaintColor,
    /// Neutral stroke style.
    pub style: PaintBorderStyle,
}

/// An outer or inset box shadow operation.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PaintShadow {
    /// Horizontal offset in CSS pixels.
    pub offset_x: f32,
    /// Vertical offset in CSS pixels.
    pub offset_y: f32,
    /// Blur radius in CSS pixels.
    pub blur: f32,
    /// Spread radius in CSS pixels.
    pub spread: f32,
    /// Shadow color.
    pub color: PaintColor,
    /// Whether this is an inset shadow.
    pub inset: bool,
}

/// A rectangular clip operation.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PaintClip {
    /// Clip rectangle in page-local coordinates.
    pub rect: PaintRect,
}

/// An image draw operation referring to an encoded image resource.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PaintImage {
    /// Resource identity for the image bytes.
    pub resource: PaintResourceId,
    /// Additional opacity applied by the producer.
    pub opacity: f32,
}

/// Neutral paint operation kind.
///
/// The vector of operations, rather than this enum's source node IDs or the
/// resource table order, is authoritative for back-to-front paint order.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PagePaintKind {
    /// A pre-shaped glyph run.
    GlyphRun(PaintGlyphRun),
    /// A solid fill.
    Fill(PaintFill),
    /// A border.
    Border(PaintBorder),
    /// A box shadow.
    Shadow(PaintShadow),
    /// An image.
    Image(PaintImage),
    /// Push a rectangular clip state.
    PushClip(PaintClip),
    /// Pop the most recent clip state.
    PopClip,
}

/// One ordered page-local paint operation.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PagePaintOperation {
    /// Monotonic order assigned by [`PagePaintPayload::push`].
    pub order: u64,
    /// Optional source node identity. Canvas/page operations use `None`.
    pub node_id: Option<NodeId>,
    /// Optional placement ordinal for correlating split/repeated content.
    pub fragment_index: Option<u32>,
    /// Physical page-local bounds used for culling and diagnostics.
    pub bounds: PaintRect,
    /// Per-operation affine transform.
    pub transform: PaintTransform,
    /// Operation-specific neutral payload.
    pub kind: PagePaintKind,
}

impl PagePaintOperation {
    /// Construct an operation before it is inserted into a payload.
    pub fn new(bounds: PaintRect, kind: PagePaintKind) -> Self {
        Self {
            order: 0,
            node_id: None,
            fragment_index: None,
            bounds,
            transform: PaintTransform::identity(),
            kind,
        }
    }

    /// Associate an operation with a source node.
    pub fn with_node_id(mut self, node_id: NodeId) -> Self {
        self.node_id = Some(node_id);
        self
    }

    /// Associate an operation with a split/repeated placement ordinal.
    pub fn with_fragment_index(mut self, fragment_index: u32) -> Self {
        self.fragment_index = Some(fragment_index);
        self
    }

    /// Set the operation's affine transform.
    pub fn with_transform(mut self, transform: PaintTransform) -> Self {
        self.transform = transform;
        self
    }
}

/// Owned neutral paint payload for one page.
///
/// `page_index` correlates this payload with a [`crate::PageFragment`], but
/// the payload's operations are already in physical page coordinates and do
/// not inherit the geometry stream's content-box-relative item contract.
/// Page size, margins, and content-box metadata remain in the geometry stream;
/// this type intentionally does not duplicate them. The producer must append
/// operations in finalized back-to-front order (not NodeId or map order),
/// preserving its page/canvas background, page furniture, and document-walk
/// sequence. `resources` is shared by
/// `Arc` so a document-level producer can reuse font and image bytes across
/// page payloads without borrowing its DOM or cache.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PagePaintPayload {
    /// Zero-based page index.
    pub page_index: u32,
    /// Paint operations in consumer application order.
    pub operations: Vec<PagePaintOperation>,
    /// Owned resource bundle retained by this payload.
    pub resources: Arc<PaintResourceBundle>,
}

impl PagePaintPayload {
    /// Construct an empty payload with an empty resource bundle.
    pub fn new(page_index: u32) -> Self {
        Self::with_resources(page_index, PaintResourceBundle::new())
    }

    /// Construct an empty payload with producer-owned resources.
    pub fn with_resources(page_index: u32, resources: PaintResourceBundle) -> Self {
        Self {
            page_index,
            operations: Vec::new(),
            resources: Arc::new(resources),
        }
    }

    /// Append an operation and assign its paint-order number.
    pub fn push(&mut self, mut operation: PagePaintOperation) {
        operation.order = self.operations.len() as u64;
        self.operations.push(operation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::PagePaintSink;

    struct RecordingPaintSink {
        pages: Vec<PagePaintPayload>,
        finished: bool,
    }

    impl PagePaintSink for RecordingPaintSink {
        fn accept_paint(&mut self, payload: PagePaintPayload) -> std::io::Result<()> {
            self.pages.push(payload);
            Ok(())
        }

        fn finish_paint(&mut self) -> std::io::Result<()> {
            self.finished = true;
            Ok(())
        }
    }

    #[test]
    fn payload_keeps_physical_coordinates_and_operation_order() {
        let font_id = PaintResourceId::new(7);
        let image_id = PaintResourceId::new(8);
        let mut resources = PaintResourceBundle::new();
        resources.push(PaintResource::font(
            font_id,
            Some("Test Sans".to_owned()),
            0,
            vec![1_u8, 2, 3],
        ));
        resources.push(PaintResource::image(image_id, "image/png", vec![4_u8, 5]));

        let mut payload = PagePaintPayload::with_resources(2, resources);
        payload.push(
            PagePaintOperation::new(
                PaintRect::new(10.0, 20.0, 30.0, 40.0),
                PagePaintKind::Fill(PaintFill {
                    color: PaintColor::new(1.0, 0.0, 0.0, 1.0),
                }),
            )
            .with_node_id(NodeId::new(11)),
        );
        payload.push(
            PagePaintOperation::new(
                PaintRect::new(10.0, 20.0, 30.0, 12.0),
                PagePaintKind::GlyphRun(PaintGlyphRun {
                    font: font_id,
                    font_size: 12.0,
                    color: PaintColor::new(0.0, 0.0, 0.0, 1.0),
                    glyphs: vec![PaintGlyph::new(42, 10.0, 20.0, 8.0, 0.0)],
                }),
            )
            .with_fragment_index(4)
            .with_transform(PaintTransform::new(1.0, 0.0, 0.0, 1.0, 2.0, 3.0)),
        );
        payload.push(PagePaintOperation::new(
            PaintRect::default(),
            PagePaintKind::PopClip,
        ));

        assert_eq!(payload.page_index, 2);
        assert_eq!(
            payload
                .operations
                .iter()
                .map(|operation| operation.order)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(payload.operations[0].bounds.x, 10.0);
        assert_eq!(payload.operations[1].transform.e, 2.0);
        assert_eq!(payload.operations[1].fragment_index, Some(4));
        assert_eq!(payload.resources.resources.len(), 2);
        assert!(matches!(payload.operations[0].kind, PagePaintKind::Fill(_)));
        assert!(matches!(
            payload.operations[1].kind,
            PagePaintKind::GlyphRun(_)
        ));
        assert!(matches!(payload.operations[2].kind, PagePaintKind::PopClip));
        assert_eq!(image_id, payload.resources.resources[1].id);
    }

    #[test]
    fn payload_new_and_initial_operation_kinds_are_constructible() {
        let empty = PagePaintPayload::new(5);
        assert!(empty.operations.is_empty());
        assert!(empty.resources.resources.is_empty());
        assert_eq!(PaintTransform::default(), PaintTransform::identity());

        let mut payload = PagePaintPayload::new(5);
        let rect = PaintRect::new(0.0, 0.0, 10.0, 10.0);
        let color = PaintColor::new(0.0, 0.0, 0.0, 1.0);
        let resource = PaintResourceId::new(12);
        let kinds = [
            PagePaintKind::PushClip(PaintClip { rect }),
            PagePaintKind::Border(PaintBorder {
                widths: PaintInsets::new(1.0, 2.0, 3.0, 4.0),
                color,
                style: PaintBorderStyle::Solid,
            }),
            PagePaintKind::Shadow(PaintShadow {
                offset_x: 1.0,
                offset_y: 2.0,
                blur: 3.0,
                spread: 4.0,
                color,
                inset: false,
            }),
            PagePaintKind::Image(PaintImage {
                resource,
                opacity: 1.0,
            }),
            PagePaintKind::PopClip,
        ];
        for kind in kinds {
            payload.push(PagePaintOperation::new(rect, kind));
        }

        assert_eq!(
            payload
                .operations
                .iter()
                .map(|operation| operation.order)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4]
        );
        assert!(matches!(
            payload.operations[0].kind,
            PagePaintKind::PushClip(_)
        ));
        assert!(matches!(
            payload.operations[1].kind,
            PagePaintKind::Border(_)
        ));
        assert!(matches!(
            payload.operations[2].kind,
            PagePaintKind::Shadow(_)
        ));
        assert!(matches!(
            payload.operations[3].kind,
            PagePaintKind::Image(_)
        ));
    }

    #[test]
    fn separate_paint_sink_accepts_owned_page_payload() {
        let mut sink = RecordingPaintSink {
            pages: Vec::new(),
            finished: false,
        };
        sink.accept_paint(PagePaintPayload::new(6))
            .expect("recording paint sink should accept payload");
        sink.finish_paint()
            .expect("recording paint sink should finish");
        assert_eq!(sink.pages.len(), 1);
        assert_eq!(sink.pages[0].page_index, 6);
        assert!(sink.finished);
    }

    #[test]
    fn cloned_payload_keeps_owned_resource_bytes_alive() {
        let mut resources = PaintResourceBundle::new();
        resources.push(PaintResource::image(
            PaintResourceId::new(9),
            "image/png",
            vec![9_u8, 8, 7],
        ));
        let payload = PagePaintPayload::with_resources(0, resources);
        let retained = payload.clone();
        drop(payload);

        assert!(matches!(
            &retained.resources.resources[0].kind,
            PaintResourceKind::Image { .. }
        ));
        let other = PaintResource::font(PaintResourceId::new(10), None, 0, vec![1_u8]);
        for kind in [&retained.resources.resources[0].kind, &other.kind] {
            if let PaintResourceKind::Image { bytes, .. } = kind {
                assert_eq!(&bytes[..], &[9_u8, 8, 7]);
            }
        }
    }
}
