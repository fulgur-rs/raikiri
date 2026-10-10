//! The heap a node's computed values hold of their own, which the cascade
//! counts against [`CascadeLimits::max_output_bytes`].
//!
//! A value counts unless the node shares it with its parent's computed values:
//! an inherited list, an `Arc` the parent holds too, or a string in the same
//! buffer. A list a node takes from a declaration it does not share with its
//! parent counts too, although the rule tree holds it as well, so the count
//! is an upper bound of what the node adds. Shared values are recognized by
//! identity before anything is walked, so an inherited list costs nothing per
//! node, and a list of the node's own is walked once, as it was built.
//!
//! [`CascadeLimits::max_output_bytes`]: super::CascadeLimits::max_output_bytes

use std::mem::size_of;
use std::sync::Arc;

use smol_str::SmolStr;

use crate::ComputedValues;
use crate::computed::CustomPropertyEnvironment;
use crate::property::{
    BackgroundImage, BasicShape, ClipPath, ContentComponent, CounterStyle, FilterFunction,
    FontFeatureSettings, FontLanguageOverride, FontPaletteValue, FontVariationSettings, Gradient,
    GridLineValue, GridTemplateAreasValue, HyphenateCharacter, ListStyleType, PositionValue,
    TextEmphasisStyle,
};
use crate::resolve::{ComputedGridTemplateTracks, ComputedGridTrackListComponent};

/// The heap bytes `values` hold that they do not share with `parent`.
///
/// Every field is named, so that a field added to [`ComputedValues`] has to
/// be sorted into those that hold heap and those that do not. Fields bound
/// to `_` hold none, or only a font key, which shares the node's
/// `font_family` list.
pub(crate) fn own_heap_bytes(values: &ComputedValues, parent: &ComputedValues) -> u64 {
    let ComputedValues {
        color: _,
        background_color: _,
        background_color_expression,
        font_family,
        font_size: _,
        font_weight: _,
        line_height: _,
        display: _,
        list_style_type,
        list_style_position: _,
        list_style_image,
        counter_reset,
        counter_increment,
        counter_set,
        content,
        string_set,
        running_templates,
        position,
        text_align: _,
        hanging_punctuation: _,
        text_autospace: _,
        word_space_transform: _,
        text_spacing_trim: _,
        text_justify: _,
        text_align_last: _,
        direction: _,
        writing_mode: _,
        cssom_writing_mode: _,
        ruby_position: _,
        text_indent: _,
        text_indent_ch_factor: _,
        text_indent_ch_offset: _,
        text_indent_ch_font: _,
        text_indent_ch_inherited: _,
        text_indent_hanging: _,
        text_indent_each_line: _,
        padding: _,
        padding_ch: _,
        margin: _,
        margin_ch: _,
        border: _,
        border_radius: _,
        box_shadow,
        outline: _,
        outline_offset: _,
        width: _,
        width_ch: _,
        height: _,
        height_ch: _,
        max_width: _,
        max_width_ch: _,
        max_height: _,
        max_height_ch: _,
        min_width: _,
        min_width_ch: _,
        min_height: _,
        min_height_ch: _,
        min_block_size: _,
        min_block_size_ch: _,
        vertical_logical_size: _,
        top: _,
        right: _,
        bottom: _,
        left: _,
        box_sizing: _,
        overflow: _,
        text_decoration_line: _,
        text_decoration_style: _,
        text_decoration_color: _,
        text_decoration_thickness: _,
        text_decoration_skip_ink: _,
        text_decoration_skip_spaces: _,
        text_decoration_inset: _,
        text_decoration_inset_start_ch: _,
        text_decoration_inset_end_ch: _,
        text_underline_offset: _,
        text_underline_position: _,
        text_emphasis_position: _,
        text_emphasis_style,
        text_emphasis_color: _,
        vertical_align: _,
        font_style: _,
        font_kerning: _,
        font_optical_sizing: _,
        font_variant_emoji: _,
        font_language_override,
        font_variant_ligatures: _,
        font_synthesis: _,
        font_variant_position: _,
        font_palette,
        font_variant_numeric: _,
        font_variant_east_asian: _,
        font_variation_settings,
        font_feature_settings,
        font_variant_caps: _,
        text_transform: _,
        text_combine_upright: _,
        text_orientation: _,
        unicode_bidi: _,
        visibility: _,
        z_index: _,
        word_break: _,
        line_break: _,
        overflow_wrap: _,
        letter_spacing: _,
        letter_spacing_computed: _,
        letter_spacing_ch_factor: _,
        letter_spacing_ch_offset: _,
        letter_spacing_ch_font: _,
        word_spacing: _,
        word_spacing_computed: _,
        word_spacing_ch_factor: _,
        word_spacing_ch_offset: _,
        word_spacing_ch_font: _,
        tab_size: _,
        break_before: _,
        break_after: _,
        break_inside: _,
        float: _,
        clear: _,
        white_space: _,
        white_space_collapse: _,
        text_wrap: _,
        text_wrap_style: _,
        effective_white_space_collapse: _,
        effective_text_wrap_mode: _,
        hyphens: _,
        hyphenate_character,
        hyphenate_limit_chars: _,
        flex_direction: _,
        flex_wrap: _,
        flex_grow: _,
        flex_shrink: _,
        flex_basis: _,
        order: _,
        justify_content: _,
        align_content: _,
        align_items: _,
        align_self: _,
        row_gap: _,
        column_gap: _,
        quotes,
        quotes_auto: _,
        text_shadow,
        grid_template_columns,
        grid_template_rows,
        grid_template_areas,
        grid_auto_columns,
        grid_auto_rows,
        grid_auto_flow: _,
        grid_row_start,
        grid_row_end,
        grid_column_start,
        grid_column_end,
        justify_items: _,
        justify_self: _,
        orphans: _,
        widows: _,
        background_repeat: _,
        background_attachment: _,
        background_clip: _,
        background_origin: _,
        background_size: _,
        background_position: _,
        background_image,
        object_fit: _,
        object_position: _,
        opacity: _,
        isolation: _,
        mix_blend_mode: _,
        mask_image,
        clip_path,
        transform,
        transform_origin: _,
        transform_origin_z: _,
        filter,
        table_layout: _,
        text_overflow: _,
        border_collapse: _,
        border_spacing: _,
        caption_side: _,
        empty_cells: _,
        column_count: _,
        column_fill: _,
        column_span: _,
        column_width: _,
        column_rule: _,
        custom_properties,
        local_custom_properties,
    } = values;
    environment(custom_properties, &parent.custom_properties)
        // The local environment is the effective one when the node declares
        // custom properties, and is counted once then.
        + if Arc::ptr_eq(local_custom_properties, custom_properties) {
            0
        } else {
            environment(local_custom_properties, &parent.local_custom_properties)
        }
        + str_of(
            background_color_expression.as_ref(),
            parent.background_color_expression.as_ref(),
        )
        + list(font_family, &parent.font_family, |family| str_heap(&family.0.0))
        + str_of(list_style_type_str(list_style_type), list_style_type_str(&parent.list_style_type))
        + image(list_style_image, &parent.list_style_image)
        + list(counter_reset, &parent.counter_reset, |(name, _)| str_heap(name))
        + list(counter_increment, &parent.counter_increment, |(name, _)| str_heap(name))
        + list(counter_set, &parent.counter_set, |(name, _)| str_heap(name))
        + list(content, &parent.content, content_component)
        + list(string_set, &parent.string_set, |(name, components)| str_heap(name) + vec_heap(components, content_component))
        + vec_heap(running_templates, |template| str_heap(&template.name))
        + str_of(position_str(position), position_str(&parent.position))
        + list(box_shadow, &parent.box_shadow, |_| 0)
        + str_of(emphasis_str(text_emphasis_style), emphasis_str(&parent.text_emphasis_style))
        + str_of(language_str(font_language_override), language_str(&parent.font_language_override))
        + str_of(palette_str(font_palette), palette_str(&parent.font_palette))
        + variations(font_variation_settings, &parent.font_variation_settings)
        + features(font_feature_settings, &parent.font_feature_settings)
        + str_of(hyphenate_str(hyphenate_character), hyphenate_str(&parent.hyphenate_character))
        + list(quotes, &parent.quotes, |(open, close)| str_heap(open) + str_heap(close))
        + list(text_shadow, &parent.text_shadow, |_| 0)
        + tracks(grid_template_columns, &parent.grid_template_columns)
        + tracks(grid_template_rows, &parent.grid_template_rows)
        + areas(grid_template_areas, &parent.grid_template_areas)
        + list(grid_auto_columns, &parent.grid_auto_columns, |_| 0)
        + list(grid_auto_rows, &parent.grid_auto_rows, |_| 0)
        + str_of(line_str(grid_row_start), line_str(&parent.grid_row_start))
        + str_of(line_str(grid_row_end), line_str(&parent.grid_row_end))
        + str_of(line_str(grid_column_start), line_str(&parent.grid_column_start))
        + str_of(line_str(grid_column_end), line_str(&parent.grid_column_end))
        + image(background_image, &parent.background_image)
        + image(mask_image, &parent.mask_image)
        + clip_path_heap(clip_path)
        + list(transform, &parent.transform, |_| 0)
        + list(filter, &parent.filter, |filter| match filter { FilterFunction::Url(url) => url.capacity() as u64, _ => 0 })
}

/// The heap of a string, which a short one keeps inline.
fn str_heap(value: &SmolStr) -> u64 {
    if value.is_heap_allocated() {
        value.len() as u64
    } else {
        0
    }
}

/// The heap of `value`, unless it shares its buffer with `parent`.
fn str_of(value: Option<&SmolStr>, parent: Option<&SmolStr>) -> u64 {
    match (value, parent) {
        (Some(value), Some(parent)) if value.as_ptr() == parent.as_ptr() => 0,
        (Some(value), _) => str_heap(value),
        (None, _) => 0,
    }
}

/// The heap of a list, unless it is the one `parent` holds: its buffer and
/// what each entry holds, as `item` measures it.
fn list<T>(value: &Arc<Vec<T>>, parent: &Arc<Vec<T>>, item: impl Fn(&T) -> u64) -> u64 {
    if Arc::ptr_eq(value, parent) {
        return 0;
    }
    vec_heap(value, item)
}

/// The heap of a vector: its whole buffer, spare capacity included, and
/// what each entry holds.
fn vec_heap<T>(value: &Vec<T>, item: impl Fn(&T) -> u64) -> u64 {
    (value.capacity() * size_of::<T>()) as u64 + value.iter().map(item).sum::<u64>()
}

/// The heap of a shared slice, whose allocation holds exactly its entries.
fn slice_heap<T>(value: &[T], item: impl Fn(&T) -> u64) -> u64 {
    std::mem::size_of_val(value) as u64 + value.iter().map(item).sum::<u64>()
}

/// The name a counter style refers to, when it names one.
fn counter_style(style: &CounterStyle) -> u64 {
    match style {
        CounterStyle::Named(name) => str_heap(name),
        CounterStyle::Decimal => 0,
    }
}

fn content_component(component: &ContentComponent) -> u64 {
    match component {
        ContentComponent::Literal(text) => str_heap(text),
        ContentComponent::Counter { name, style } => str_heap(name) + counter_style(style),
        ContentComponent::String { name, .. }
        | ContentComponent::Element { name, .. }
        | ContentComponent::Attr { name } => str_heap(name),
        ContentComponent::Counters {
            name,
            separator,
            style,
        } => str_heap(name) + separator.capacity() as u64 + counter_style(style),
        ContentComponent::AttrFallback { name, fallback } => {
            str_heap(name) + fallback.as_ref().map_or(0, str_heap)
        }
        ContentComponent::TargetCounter { url, name, style } => {
            url.capacity() as u64 + str_heap(name) + counter_style(style)
        }
        ContentComponent::TargetCounters {
            url,
            name,
            separator,
            style,
        } => {
            url.capacity() as u64
                + str_heap(name)
                + separator.capacity() as u64
                + counter_style(style)
        }
        ContentComponent::TargetText { url, .. } | ContentComponent::Image { url } => {
            url.capacity() as u64
        }
        _ => 0,
    }
}

fn image(value: &BackgroundImage, parent: &BackgroundImage) -> u64 {
    match (value, parent) {
        (BackgroundImage::Url(url), BackgroundImage::Url(parent)) => {
            str_of(Some(url), Some(parent))
        }
        (BackgroundImage::Url(url), _) => str_heap(url),
        (BackgroundImage::Gradient(gradient), BackgroundImage::Gradient(parent))
            if Arc::ptr_eq(gradient, parent) =>
        {
            0
        }
        (BackgroundImage::Gradient(gradient), _) => {
            size_of::<Gradient>() as u64
                + match &**gradient {
                    Gradient::Linear(linear) => vec_heap(&linear.stops, |_| 0),
                    Gradient::Radial(radial) => vec_heap(&radial.stops, |_| 0),
                    Gradient::Conic(conic) => vec_heap(&conic.stops, |_| 0),
                }
        }
        _ => 0,
    }
}

fn features(value: &FontFeatureSettings, parent: &FontFeatureSettings) -> u64 {
    match (value, parent) {
        (FontFeatureSettings::Features(list), FontFeatureSettings::Features(parent))
            if list.shares(parent) =>
        {
            0
        }
        (FontFeatureSettings::Features(list), _) => slice_heap(list, |_| 0),
        _ => 0,
    }
}

fn variations(value: &FontVariationSettings, parent: &FontVariationSettings) -> u64 {
    match (value, parent) {
        (FontVariationSettings::Settings(list), FontVariationSettings::Settings(parent))
            if list.shares(parent) =>
        {
            0
        }
        (FontVariationSettings::Settings(list), _) => {
            slice_heap(list, |setting| str_heap(&setting.tag))
        }
        _ => 0,
    }
}

fn tracks(value: &ComputedGridTemplateTracks, parent: &ComputedGridTemplateTracks) -> u64 {
    let names = |lines: &Vec<Vec<SmolStr>>| vec_heap(lines, |names| vec_heap(names, str_heap));
    match (value, parent) {
        (ComputedGridTemplateTracks::List(list), ComputedGridTemplateTracks::List(parent))
            if Arc::ptr_eq(list, parent) =>
        {
            0
        }
        (ComputedGridTemplateTracks::List(list), _) => {
            names(&list.line_names)
                + vec_heap(&list.components, |component| match component {
                    ComputedGridTrackListComponent::Repeat(repeat) => {
                        names(&repeat.line_names) + vec_heap(&repeat.tracks, |_| 0)
                    }
                    ComputedGridTrackListComponent::Size(_) => 0,
                })
        }
        _ => 0,
    }
}

fn areas(value: &GridTemplateAreasValue, parent: &GridTemplateAreasValue) -> u64 {
    match (value, parent) {
        (GridTemplateAreasValue::Areas(areas), GridTemplateAreasValue::Areas(parent))
            if Arc::ptr_eq(areas, parent) =>
        {
            0
        }
        (GridTemplateAreasValue::Areas(areas), _) => {
            vec_heap(&areas.row_strings, str_heap)
                + vec_heap(&areas.areas, |area| str_heap(&area.name))
        }
        _ => 0,
    }
}

/// `clip-path` is not inherited, and the node holds its own copy.
fn clip_path_heap(value: &ClipPath) -> u64 {
    match value {
        ClipPath::Url(url) => url.capacity() as u64,
        ClipPath::BasicShape { shape, .. } => {
            size_of::<BasicShape>() as u64
                + match &**shape {
                    BasicShape::Polygon(polygon) => vec_heap(&polygon.points, |_| 0),
                    BasicShape::Path(path) => path.path.capacity() as u64,
                    _ => 0,
                }
        }
        _ => 0,
    }
}

fn environment(
    value: &Arc<CustomPropertyEnvironment>,
    parent: &Arc<CustomPropertyEnvironment>,
) -> u64 {
    if Arc::ptr_eq(value, parent) {
        0
    } else {
        value.local_heap_bytes()
    }
}

fn list_style_type_str(value: &ListStyleType) -> Option<&SmolStr> {
    match value {
        ListStyleType::Named(name) | ListStyleType::String(name) => Some(name),
        _ => None,
    }
}

fn position_str(value: &PositionValue) -> Option<&SmolStr> {
    match value {
        PositionValue::Running(name) => Some(name),
        _ => None,
    }
}

fn emphasis_str(value: &TextEmphasisStyle) -> Option<&SmolStr> {
    match value {
        TextEmphasisStyle::String(text) => Some(text),
        _ => None,
    }
}

fn language_str(value: &FontLanguageOverride) -> Option<&SmolStr> {
    match value {
        FontLanguageOverride::String(text) => Some(text),
        _ => None,
    }
}

fn palette_str(value: &FontPaletteValue) -> Option<&SmolStr> {
    match value {
        FontPaletteValue::Palette(name) => Some(name),
        _ => None,
    }
}

fn hyphenate_str(value: &HyphenateCharacter) -> Option<&SmolStr> {
    match value {
        HyphenateCharacter::String(text) => Some(text),
        _ => None,
    }
}

fn line_str(value: &GridLineValue) -> Option<&SmolStr> {
    match value {
        GridLineValue::Named(name)
        | GridLineValue::NamedLine(name, _)
        | GridLineValue::SpanNamed(name, _) => Some(name),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
