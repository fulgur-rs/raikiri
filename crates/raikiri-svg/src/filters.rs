//! The cost of a tree's filters, checked before resvg runs them.
//!
//! resvg applies a filter on a layer as large as the filter region, which it
//! lets grow to five canvases on each side, and keeps the result of every
//! primitive until the filter is done. Some primitives also take time that
//! grows with a number the document sets: the radius of a morphology, the size
//! of a convolution matrix, the octaves of turbulence. The output byte limit
//! bounds neither, so a small document could ask for gigabytes or hours. The
//! check estimates both from the parsed tree and the transform it is drawn with,
//! and fails the picture over a budget instead of starting.

use crate::{DEFAULT_MAX_OUTPUT_BYTES, SvgError, SvgFilterBudget};
use usvg::filter::{EdgeMode, Filter, Kind};
use usvg::{Fill, Stroke};

/// Most pixels one filter may hold at once: eight output-sized buffers.
const MAX_FILTER_PIXELS: u64 = 8 * (DEFAULT_MAX_OUTPUT_BYTES / 4);
/// Most work all the filters of one picture may do, in pixel operations. A
/// blur or a flood over a layer is one operation per pixel.
const MAX_FILTER_WORK: u64 = 1 << 32;
/// Pixel operations of one octave of turbulence at one pixel.
const OCTAVE_WORK: f64 = 16.0;
/// How many canvases wide and high resvg lets a filter layer be.
const LAYER_CANVASES: f64 = 5.0;

/// Fails when the filters of `tree` drawn with `transform` on a canvas of
/// `canvas` pixels would ask for more than their budgets.
pub(crate) fn check(
    tree: &usvg::Tree,
    transform: usvg::Transform,
    canvas: (u32, u32),
) -> Result<(), SvgError> {
    Meter {
        canvas: (f64::from(canvas.0), f64::from(canvas.1)),
        work: 0.0,
    }
    .group(tree.root(), transform)
}

/// The layer resvg draws a group with filters on.
#[derive(Clone, Copy)]
struct Layer {
    /// Size in canvas pixels.
    width: f64,
    height: f64,
    /// Canvas pixels per unit of the group's coordinates.
    scale: (f64, f64),
    /// Places the group's coordinates on the canvas.
    to_canvas: usvg::Transform,
}

struct Meter {
    canvas: (f64, f64),
    /// What the filters met so far ask for, in pixel operations.
    work: f64,
}

impl Meter {
    /// Charges the filters in `group` and in the subroots below it. `base`
    /// takes the coordinates of the root that holds `group` to the canvas.
    fn group(&mut self, group: &usvg::Group, base: usvg::Transform) -> Result<(), SvgError> {
        for node in group.children() {
            let to_canvas = base.pre_concat(node.abs_transform());
            // Only these kinds hold a filter on the raster path: it is given
            // no picture, and resvg draws a text as the paths it flattens to.
            if let usvg::Node::Group(inner) = node {
                // resvg draws all the filters of a group on one layer, the
                // union of their regions, and each primitive handles the whole
                // of it.
                let layer = self.extent(inner.layer_bounding_box(), to_canvas);
                for filter in inner.filters() {
                    self.filter(filter, layer)?;
                }
                self.group(inner, base)?;
                // A mask can link to another without limit, and resvg draws
                // the whole chain. A clip path holds no filter: usvg drops
                // them there.
                let mut mask = inner.mask();
                while let Some(link) = mask {
                    self.group(link.root(), to_canvas)?;
                    mask = link.mask();
                }
                // The picture of an `feImage` holds nodes of its own.
                for filter in inner.filters() {
                    for primitive in filter.primitives() {
                        if let Kind::Image(image) = primitive.kind() {
                            self.at_scale(image.root(), to_canvas)?;
                        }
                    }
                }
            }
            if let usvg::Node::Path(path) = node {
                let paints = [
                    path.fill().map(Fill::paint),
                    path.stroke().map(Stroke::paint),
                ];
                for paint in paints.into_iter().flatten() {
                    if let usvg::Paint::Pattern(pattern) = paint {
                        self.at_scale(pattern.root(), to_canvas.pre_concat(pattern.transform()))?;
                    }
                }
            }
            if let usvg::Node::Text(text) = node {
                self.group(text.flattened(), base)?;
            }
        }
        Ok(())
    }

    /// Charges the filters in `root`, which resvg draws at the scale of
    /// `to_canvas` and at no other part of it: the tile of a pattern and the
    /// picture of an `feImage` are drawn that way.
    fn at_scale(&mut self, root: &usvg::Group, to_canvas: usvg::Transform) -> Result<(), SvgError> {
        let (scale_x, scale_y) = to_canvas.get_scale();
        self.group(root, usvg::Transform::from_scale(scale_x, scale_y))
    }

    /// The pixels that resvg draws for `rect`, an area in the coordinates of a
    /// group, when `to_canvas` places the group on the canvas.
    fn extent(&self, rect: usvg::NonZeroRect, to_canvas: usvg::Transform) -> Layer {
        let (width, height) = rect.transform(to_canvas).map_or((0.0, 0.0), |region| {
            // resvg rounds the region out to whole pixels, and to at least one.
            (
                f64::from(region.width())
                    .ceil()
                    .clamp(1.0, LAYER_CANVASES * self.canvas.0),
                f64::from(region.height())
                    .ceil()
                    .clamp(1.0, LAYER_CANVASES * self.canvas.1),
            )
        });
        let (scale_x, scale_y) = to_canvas.get_scale();
        Layer {
            width,
            height,
            scale: (f64::from(scale_x), f64::from(scale_y)),
            to_canvas,
        }
    }

    /// Charges `filter`, which resvg applies to `layer`.
    fn filter(&mut self, filter: &Filter, layer: Layer) -> Result<(), SvgError> {
        let area = layer.width * layer.height;
        let primitives = filter.primitives();
        // One buffer for each result, and one for the source graphic.
        let pixels = area * (primitives.len() + 1) as f64;
        if pixels > MAX_FILTER_PIXELS as f64 {
            return Err(SvgError::FilterLimitExceeded {
                budget: SvgFilterBudget::Pixels,
                requested: pixels as u64,
                limit: MAX_FILTER_PIXELS,
            });
        }
        // The results of the primitives have at least the size of the region
        // of the filter, which lies within the layer.
        let region = self.extent(filter.rect(), layer.to_canvas);
        for primitive in primitives {
            self.work +=
                area * per_pixel_work(primitive.kind(), layer, (region.width, region.height));
        }
        if self.work > MAX_FILTER_WORK as f64 {
            return Err(SvgError::FilterLimitExceeded {
                budget: SvgFilterBudget::Work,
                requested: self.work as u64,
                limit: MAX_FILTER_WORK,
            });
        }
        Ok(())
    }
}

/// Pixel operations `kind` spends on each pixel of `layer`, where the images
/// it handles are at least `input` pixels wide and high.
fn per_pixel_work(kind: &Kind, layer: Layer, input: (f64, f64)) -> f64 {
    let work = match kind {
        // Every pixel is compared with a kernel of twice the radius on a
        // side, which resvg keeps within the layer.
        Kind::Morphology(fe) => {
            (2.0 * (f64::from(fe.radius_x().get()) * layer.scale.0).ceil()).min(layer.width)
                * (2.0 * (f64::from(fe.radius_y().get()) * layer.scale.1).ceil()).min(layer.height)
        }
        Kind::ConvolveMatrix(fe) => {
            let matrix = fe.matrix();
            // In wrap mode resvg brings a coordinate back into the image by
            // adding the size of the image once per step, so each cell of the
            // kernel takes a step for every image size that the target lies
            // from the edge.
            let steps = if matches!(fe.edge_mode(), EdgeMode::Wrap) {
                1.0 + (f64::from(matrix.target_x()) / input.0).ceil()
                    + (f64::from(matrix.target_y()) / input.1).ceil()
            } else {
                1.0
            };
            f64::from(matrix.columns()) * f64::from(matrix.rows()) * steps
        }
        Kind::Turbulence(fe) => f64::from(fe.num_octaves()) * OCTAVE_WORK,
        // Every input is converted and composited over the result.
        Kind::Merge(fe) => fe.inputs().len() as f64,
        _ => 1.0,
    };
    work.max(1.0)
}
