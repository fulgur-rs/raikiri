use super::ReadRole;
use crate::{LayoutStyle, layout::context::LayoutSession};
use taffy::{BlockContainerStyle, BlockItemStyle, CoreStyle, OofItemStyle};

#[derive(Clone, Copy)]
pub(crate) struct LegacyQuery<'a> {
    style: &'a LayoutStyle,
    padding: taffy::Rect<taffy::LengthPercentage>,
    border: taffy::Rect<taffy::LengthPercentage>,
    claims: Option<taffy::ContainingBlockClaims>,
    oof_table: bool,
}
impl<'a> LegacyQuery<'a> {
    pub(crate) fn plain(style: &'a LayoutStyle) -> Self {
        Self {
            style,
            padding: style.padding,
            border: style.border,
            claims: None,
            oof_table: style.item_is_table,
        }
    }
    pub(crate) fn style(&self) -> &'a LayoutStyle {
        self.style
    }
    pub(crate) fn for_role(tree: &'a LayoutSession<'_>, idx: usize, role: ReadRole) -> Self {
        let base = tree
            .layout_style_for_key(idx)
            .expect("legacy oracle has a retained style");
        let transposed = match role {
            ReadRole::BlockContainer => tree.transposed_container_style(idx),
            ReadRole::BlockItem | ReadRole::Oof => tree.transposed_child_style(idx),
            _ => None,
        };
        let mut query = Self::plain(transposed.unwrap_or(base));
        if matches!(role, ReadRole::CoreContainer) {
            query.claims = Some(taffy::ContainingBlockClaims {
                absolute: tree.nodes[idx].claims_absolute_oof || base.position.is_positioned(),
                fixed: tree.nodes[idx].claims_fixed_oof,
            });
        }
        if matches!(role, ReadRole::Oof) {
            query.oof_table |= matches!(
                tree.nodes[idx].display(),
                raikiri_style::property::DisplayValue::Table
                    | raikiri_style::property::DisplayValue::InlineTable
            );
        }
        if matches!(role, ReadRole::BlockItem) && transposed.is_none() {
            let basis = tree
                .context
                .block_item_percent_bases
                .last()
                .filter(|&&(container, _)| {
                    tree.nodes[idx].taffy_position() != taffy::Position::Absolute
                        && tree.taffy_parent_of(idx) == Some(container)
                })
                .and_then(|&(_, basis)| basis);
            if let Some(basis) = basis {
                use taffy::ResolveOrZero;
                let resolve = |edge: taffy::LengthPercentage| {
                    taffy::LengthPercentage::length(
                        edge.resolve_or_zero(Some(basis), crate::taffy_impl::resolve_calc),
                    )
                };
                query.padding.top = resolve(query.padding.top);
                query.padding.bottom = resolve(query.padding.bottom);
                query.border.top = resolve(query.border.top);
                query.border.bottom = resolve(query.border.bottom);
            }
        }
        query
    }
}
macro_rules! delegate {
    ($trait:ident; $($method:ident: $value:ty),+ $(,)?) => { $(
        fn $method(&self) -> $value { $trait::$method(self.style) }
    )+ };
}
impl CoreStyle for LegacyQuery<'_> {
    type CustomIdent = raikiri_style::SmolStr;
    delegate!(CoreStyle;box_generation_mode: taffy::BoxGenerationMode,
is_block: bool,
is_compressible_replaced: bool,
is_replaced: bool,
box_sizing: taffy::BoxSizing,
direction: taffy::Direction,
overflow: taffy::Point<taffy::Overflow>,
scrollbar_width: f32,
position: taffy::Position,
inset: taffy::Rect<taffy::LengthPercentageAuto>,
size: taffy::Size<taffy::Dimension>,
min_size: taffy::Size<taffy::LengthPercentageAuto>,
max_size: taffy::Size<taffy::LengthPercentageAuto>,
aspect_ratio: Option<f32>,
margin: taffy::Rect<taffy::LengthPercentageAuto>,
contain: taffy::Contain);
    fn padding(&self) -> taffy::Rect<taffy::LengthPercentage> {
        self.padding
    }
    fn border(&self) -> taffy::Rect<taffy::LengthPercentage> {
        self.border
    }
    fn is_containing_block(&self) -> taffy::ContainingBlockClaims {
        self.claims
            .unwrap_or_else(|| self.style.is_containing_block())
    }
}
impl BlockContainerStyle for LegacyQuery<'_> {
    delegate!(BlockContainerStyle; text_align:taffy::TextAlign, align_content:taffy::AlignContent, justify_items:taffy::AlignItems);
}
impl BlockItemStyle for LegacyQuery<'_> {
    delegate!(BlockItemStyle; is_table:bool, float:taffy::Float, clear:taffy::Clear);
}
impl OofItemStyle for LegacyQuery<'_> {
    fn align_self(&self) -> Option<taffy::AlignSelf> {
        self.style.align_self
    }
    fn justify_self(&self) -> Option<taffy::AlignSelf> {
        self.style.justify_self
    }
    fn is_table(&self) -> bool {
        self.oof_table
    }
    fn grid_row(&self) -> taffy::Line<taffy::GridPlacement<raikiri_style::SmolStr>> {
        self.style.grid_row.clone()
    }
    fn grid_column(&self) -> taffy::Line<taffy::GridPlacement<raikiri_style::SmolStr>> {
        self.style.grid_column.clone()
    }
}
