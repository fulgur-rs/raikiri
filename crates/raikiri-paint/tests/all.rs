//! Single integration-test binary for this crate.
//!
//! Each `tests/*.rs` file used to be its own test binary. Linking several
//! hundred binaries against the whole dependency graph dominated the
//! `cargo test` build time, so `autotests = false` in `Cargo.toml` turns
//! automatic discovery off and the plain integration-test files are compiled
//! as modules of this one crate instead.
//!
//! Files that need a process of their own (a `#[global_allocator]` that
//! counts allocations, a wall-clock or peak-memory budget, `#[path]`
//! includes, or submodules) stay separate `[[test]]` targets in `Cargo.toml`.
//!
//! Add a new integration-test file as `mod <file_stem>;` below, or as its
//! own `[[test]]` target. `scripts/orphan-tests-lint.sh` fails when a
//! `tests/*.rs` file is in neither place.

mod absolute_page_clip;
mod background_tiling;
mod border_image;
mod canvas_contain_background;
mod canvas_root;
mod canvas_streaming;
mod cloned_page_edges;
mod collapsed_corners;
mod collapsed_dashed_border;
mod column_shadow_tests;
mod counter_snapshot_limit;
mod fieldset_legend;
mod first_letter;
mod first_line_style;
mod fixed_static_position_pages;
mod generated_block_box;
mod generated_box_placement;
mod generated_content;
mod hidden_list_marker;
mod inline_background_slice;
mod inline_fixed_background;
mod inline_relative_image_position;
mod inline_svg_cache;
mod inline_svg_stylesheet;
mod list_marker_page;
mod margin_box_layers;
mod marker_overflow;
mod multicol_fragments;
mod multicol_placement;
mod multicol_rules;
mod outside_marker_offset;
mod page_lines;
mod paint_order;
mod raster_image_placement;
mod raster_marker_placement;
mod rounded_border_bands;
mod rounded_dashed_border;
mod row_group_borders;
mod ruby_annotation_boxes;
mod synthetic_italic;
mod table_alignment;
mod table_anonymous_cells;
mod table_anonymous_pagination;
mod table_anonymous_paragraph;
mod table_header_repeat;
mod table_part_visibility;
mod text_autospace;
mod text_runs;
mod text_shadow_clip;
mod vertical_metrics;
mod visibility_hidden;
