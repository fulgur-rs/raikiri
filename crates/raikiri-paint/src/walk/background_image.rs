//! The tiling of a decoded URL background image (CSS Backgrounds 3 §3.4–§3.6).

use super::*;
use raikiri_dom::image_geometry::BackgroundTiles;
use raikiri_style::property::{BackgroundRepeat, BackgroundRepeatKeyword};

/// One decoded background image, tiled over the painting area of its layer.
///
/// [`paint_background_image`] describes the boxes and the repeat keywords.
pub(super) struct ImageLayer<'a> {
    pub(super) image_cache: &'a ImageBrushCache,
    pub(super) decoded: &'a std::sync::Arc<raikiri_traits::DecodedImage>,
    /// The `background-origin` box.
    pub(super) positioning: Rect,
    /// The `background-clip` box.
    pub(super) painting: Rect,
    /// The used size of a tile, before `round` rescales it.
    pub(super) image: (f64, f64),
    pub(super) position: &'a ComputedCssPosition,
    pub(super) repeat: &'a BackgroundRepeat,
    pub(super) device_grid: Option<DeviceGrid>,
}

/// The tiles of a layer along one axis.
struct AxisTiles {
    /// The `(start, length)` of each tile.
    spans: Vec<(f64, f64)>,
    /// The first of `spans`; the start of the painting area and the tile
    /// length if there is none.
    first: (f64, f64),
    /// Whether the axis repeats edge to edge (`repeat` or `round`), so one
    /// fill with the image repeating along it covers the axis without a seam
    /// between tiles, however small they are. A `space` or `no-repeat` axis
    /// keeps one fill per tile, which leaves its gaps and clips the tile to
    /// its edges.
    continuous: bool,
}

impl AxisTiles {
    /// The tiles `spans` of an axis with the repeat `keyword`. `fallback` is
    /// the first tile if `spans` is empty.
    fn new(spans: Vec<(f64, f64)>, fallback: (f64, f64), keyword: BackgroundRepeatKeyword) -> Self {
        let first = spans.first().copied().unwrap_or(fallback);
        let continuous = matches!(
            keyword,
            BackgroundRepeatKeyword::Repeat | BackgroundRepeatKeyword::Round
        );
        Self {
            spans,
            first,
            continuous,
        }
    }
}

/// The tiles of a layer along both axes.
struct TileGrid {
    x: AxisTiles,
    y: AxisTiles,
}

impl TileGrid {
    /// `brush`, repeating along the continuous axes.
    fn pattern(&self, brush: &peniko::ImageBrush) -> peniko::ImageBrush {
        let mut pattern = brush.clone();
        if self.x.continuous {
            pattern.sampler.x_extend = PenikoExtend::Repeat;
        }
        if self.y.continuous {
            pattern.sampler.y_extend = PenikoExtend::Repeat;
        }
        pattern
    }
}

impl ImageLayer<'_> {
    /// Paints the tiles, clipped to the painting area, unless the image has
    /// no pixels or no tile fits.
    pub(super) fn paint(&self, scene: &mut impl PaintScene) {
        if self.decoded.width == 0 || self.decoded.height == 0 {
            return;
        }
        let Some(tiles) = self.tiles() else {
            return;
        };
        let brush = self.image_cache.whole(self.decoded);
        let grid = TileGrid {
            x: self.x_tiles(&tiles),
            y: self.y_tiles(&tiles),
        };
        scene.push_clip_layer(Affine::IDENTITY, &self.painting);
        match (grid.x.continuous, grid.y.continuous) {
            (true, true) => self.fill_covering(scene, &grid, &brush),
            (true, false) => self.fill_rows(scene, &grid, &brush),
            (false, true) => self.fill_columns(scene, &grid, &brush),
            (false, false) => self.fill_each(scene, &grid, &brush),
        }
        scene.pop_layer();
    }

    /// Where the tiles of the layer fall.
    fn tiles(&self) -> Option<BackgroundTiles> {
        background_tiles(
            (
                self.positioning.x0,
                self.positioning.y0,
                self.positioning.width(),
                self.positioning.height(),
            ),
            (
                self.painting.x0,
                self.painting.y0,
                self.painting.width(),
                self.painting.height(),
            ),
            self.image,
            self.position,
            self.repeat,
        )
    }

    /// The tile columns, snapped to the device grid when it is known.
    fn x_tiles(&self, tiles: &BackgroundTiles) -> AxisTiles {
        let spans = axis_tile_spans(
            &tiles.x,
            tiles.width,
            self.repeat.x,
            (
                self.positioning.x0
                    + position_offset(
                        self.position.horizontal,
                        self.positioning.width() - tiles.width,
                    ),
                self.painting.x0,
            ),
            self.device_grid.map(|grid| (grid.scale_x, grid.offset_x)),
        );
        // `background_tiles` returns a tile in each axis, so the fallback
        // never applies.
        AxisTiles::new(spans, (self.painting.x0, tiles.width), self.repeat.x)
    }

    /// The tile rows, snapped to the device grid when it is known.
    fn y_tiles(&self, tiles: &BackgroundTiles) -> AxisTiles {
        let spans = axis_tile_spans(
            &tiles.y,
            tiles.height,
            self.repeat.y,
            (
                self.positioning.y0
                    + position_offset(
                        self.position.vertical,
                        self.positioning.height() - tiles.height,
                    ),
                self.painting.y0,
            ),
            self.device_grid.map(|grid| (grid.scale_y, grid.offset_y)),
        );
        AxisTiles::new(spans, (self.painting.y0, tiles.height), self.repeat.y)
    }

    /// The scale that maps the image onto a tile of the given size.
    fn tile_scale(&self, width: f64, height: f64) -> Affine {
        Affine::scale_non_uniform(
            width / f64::from(self.decoded.width),
            height / f64::from(self.decoded.height),
        )
    }

    /// Both axes repeat edge to edge: one fill of the painting area covers
    /// it.
    fn fill_covering(
        &self,
        scene: &mut impl PaintScene,
        grid: &TileGrid,
        brush: &peniko::ImageBrush,
    ) {
        let ((origin_x, tile_w), (origin_y, tile_h)) = (grid.x.first, grid.y.first);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            grid.pattern(brush).as_ref(),
            Some(Affine::translate((origin_x, origin_y)) * self.tile_scale(tile_w, tile_h)),
            &self.painting,
        );
    }

    /// Only the horizontal axis repeats edge to edge: one fill per tile row.
    fn fill_rows(&self, scene: &mut impl PaintScene, grid: &TileGrid, brush: &peniko::ImageBrush) {
        let pattern = grid.pattern(brush);
        let (origin_x, tile_w) = grid.x.first;
        for (tile_y, height) in &grid.y.spans {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                pattern.as_ref(),
                Some(Affine::translate((origin_x, *tile_y)) * self.tile_scale(tile_w, *height)),
                &Rect::new(
                    self.painting.x0,
                    *tile_y,
                    self.painting.x1,
                    *tile_y + height,
                ),
            );
        }
    }

    /// Only the vertical axis repeats edge to edge: one fill per tile column.
    fn fill_columns(
        &self,
        scene: &mut impl PaintScene,
        grid: &TileGrid,
        brush: &peniko::ImageBrush,
    ) {
        let pattern = grid.pattern(brush);
        let (origin_y, tile_h) = grid.y.first;
        for (tile_x, width) in &grid.x.spans {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                pattern.as_ref(),
                Some(Affine::translate((*tile_x, origin_y)) * self.tile_scale(*width, tile_h)),
                &Rect::new(*tile_x, self.painting.y0, *tile_x + width, self.painting.y1),
            );
        }
    }

    /// Neither axis repeats edge to edge: one fill per tile.
    fn fill_each(&self, scene: &mut impl PaintScene, grid: &TileGrid, brush: &peniko::ImageBrush) {
        let tile_shape = Rect::new(
            0.0,
            0.0,
            f64::from(self.decoded.width),
            f64::from(self.decoded.height),
        );
        for (tile_y, height) in &grid.y.spans {
            for (tile_x, width) in &grid.x.spans {
                scene.fill(
                    Fill::NonZero,
                    Affine::translate((*tile_x, *tile_y)) * self.tile_scale(*width, *height),
                    brush.as_ref(),
                    None,
                    &tile_shape,
                );
            }
        }
    }
}
