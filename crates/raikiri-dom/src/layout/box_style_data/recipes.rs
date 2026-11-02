use raikiri_layout_style::StyleMetadata;
use raikiri_layout_style::values::{Direction as LayoutDirection, Display};
use raikiri_style::property::{BorderCollapseValue, CaptionSideValue, Direction, DisplayValue};
use raikiri_style::{ComputedBorderSpacing, ComputedLengthPercentageOrAuto as L, ComputedValues};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RecipeKind {
    InitialContainingBlock { width: f32, height: f32 },
    AnonymousTable { owner: usize },
    AnonymousCell { owner: usize },
    Contents,
    Spanner,
    NeutralSource,
}

#[derive(Clone, Copy)]
pub(crate) struct BoxFacts {
    pub(crate) direction: LayoutDirection,
    pub(crate) metadata: StyleMetadata,
}

impl BoxFacts {
    pub(crate) fn for_node(node: &crate::node::Node, direction: LayoutDirection) -> Self {
        let mut flags = if node.layout_display() == Display::Block {
            raikiri_layout_style::StyleFlags::BLOCK
        } else {
            raikiri_layout_style::StyleFlags::EMPTY
        };
        if node.layout_display() == Display::None {
            flags = flags.union(raikiri_layout_style::StyleFlags::HIDDEN);
        }
        Self {
            direction,
            metadata: crate::layout::style_views::metadata::node_metadata(node, flags),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct TableFacts {
    pub(crate) owner: usize,
    pub(crate) box_facts: BoxFacts,
    pub(crate) border_collapse: BorderCollapseValue,
    pub(crate) border_spacing: ComputedBorderSpacing,
    pub(crate) caption_side: CaptionSideValue,
}

impl TableFacts {
    pub(crate) fn from_owner(
        document: &crate::Document,
        cascade: &raikiri_style::CascadeResult,
        key: usize,
        owner: usize,
    ) -> Self {
        let source = &document.nodes[owner];
        let direction = crate::layout::bridge::direction_for_axes(
            &cascade.computed[owner],
            crate::layout::bridge::uses_vertical_axes(cascade, owner),
        );
        let mut box_facts = BoxFacts::for_node(&document.nodes[key], direction);
        box_facts.metadata.used_writing_mode = source.used_writing_mode;
        Self {
            owner,
            box_facts,
            border_collapse: source.border_collapse,
            border_spacing: source.border_spacing,
            caption_side: source.caption_side,
        }
    }
}

pub(crate) struct Recipe {
    pub(crate) kind: RecipeKind,
    pub(crate) values: Arc<ComputedValues>,
    pub(crate) metadata: StyleMetadata,
}

pub(crate) fn neutral_initial_values() -> ComputedValues {
    static EMPTY_TRACKS: OnceLock<Arc<Vec<raikiri_style::ComputedGridTrackSize>>> = OnceLock::new();
    let mut values = ComputedValues::initial();
    values.box_sizing = raikiri_style::property::BoxSizing::BorderBox;
    let empty = EMPTY_TRACKS.get_or_init(|| Arc::new(Vec::new()));
    values.grid_auto_columns = empty.clone();
    values.grid_auto_rows = empty.clone();
    values
}

pub(crate) fn empty_prepared_data() -> Arc<raikiri_layout_style::PreparedData> {
    static DATA: OnceLock<Arc<raikiri_layout_style::PreparedData>> = OnceLock::new();
    DATA.get_or_init(|| {
        Arc::new(
            raikiri_layout_style::PreparedData::prepare_with(
                &neutral_initial_values(),
                // cov:ignore: neutral CSS-initial values have no calc slots, so this factory cannot run.
                |_, _, _| unreachable!("neutral initial properties have no calc slots"),
            )
            .expect("neutral initial properties have valid calc payloads"),
        )
    })
    .clone()
}

fn values(facts: BoxFacts, display: DisplayValue) -> ComputedValues {
    let mut values = neutral_initial_values();
    values.display = display;
    values.direction = match facts.direction {
        LayoutDirection::Ltr => Direction::Ltr,
        LayoutDirection::Rtl => Direction::Rtl,
    };
    values
}

pub(crate) fn initial_containing_block(width: f32, height: f32, facts: BoxFacts) -> Recipe {
    let mut values = values(facts, DisplayValue::Block);
    values.width = L::Px(width);
    values.height = L::Px(height);
    Recipe {
        kind: RecipeKind::InitialContainingBlock { width, height },
        values: Arc::new(values),
        metadata: facts.metadata,
    }
}

fn table(kind: RecipeKind, display: DisplayValue, facts: TableFacts) -> Recipe {
    let mut values = values(facts.box_facts, display);
    values.border_collapse = facts.border_collapse;
    values.border_spacing = facts.border_spacing;
    values.caption_side = facts.caption_side;
    Recipe {
        kind,
        values: Arc::new(values),
        metadata: facts.box_facts.metadata,
    }
}

pub(crate) fn anonymous_table(facts: TableFacts) -> Recipe {
    table(
        RecipeKind::AnonymousTable { owner: facts.owner },
        DisplayValue::Table,
        facts,
    )
}

pub(crate) fn anonymous_cell(facts: TableFacts) -> Recipe {
    table(
        RecipeKind::AnonymousCell { owner: facts.owner },
        DisplayValue::TableCell,
        facts,
    )
}

pub(crate) fn contents(facts: BoxFacts) -> Recipe {
    Recipe {
        kind: RecipeKind::Contents,
        values: Arc::new(values(facts, DisplayValue::Contents)),
        metadata: facts.metadata,
    }
}

pub(crate) fn spanner(facts: BoxFacts) -> Recipe {
    Recipe {
        kind: RecipeKind::Spanner,
        values: Arc::new(values(facts, DisplayValue::Contents)),
        metadata: facts.metadata,
    }
}

pub(crate) fn neutral_source(facts: BoxFacts) -> Recipe {
    static LTR: OnceLock<Arc<ComputedValues>> = OnceLock::new();
    static RTL: OnceLock<Arc<ComputedValues>> = OnceLock::new();
    let shared = match facts.direction {
        LayoutDirection::Ltr => &LTR,
        LayoutDirection::Rtl => &RTL,
    };
    Recipe {
        kind: RecipeKind::NeutralSource,
        values: shared
            .get_or_init(|| Arc::new(values(facts, DisplayValue::Inline)))
            .clone(),
        metadata: facts.metadata,
    }
}

pub(crate) fn refresh(
    document: &crate::Document,
    cascade: &raikiri_style::CascadeResult,
    key: crate::node::arena::BoxKey,
    index: usize,
    record: &super::BoxStyleData,
) -> Option<Recipe> {
    use crate::node::arena::BoxKey;
    let old_kind = record.recipe.expect("synthetic recipe has explicit inputs");
    let super::LayoutBase::Synthetic(values) = &record.base else {
        panic!("recipe input belongs to a synthetic base")
    };
    let (kind, facts, table_facts) = match old_kind {
        RecipeKind::InitialContainingBlock { .. } => {
            assert_eq!(key, BoxKey::InitialContainingBlock);
            (
                old_kind,
                crate::layout::initial_containing_block::recipe_facts(document, cascade),
                None,
            )
        }
        RecipeKind::AnonymousTable { owner } => {
            let BoxKey::AnonymousTable(slot) = key else {
                panic!("anonymous table identity")
            };
            let owner = document
                .table_objects
                .tables
                .get(slot)
                .map_or(owner, |table| table.parent);
            let facts = TableFacts::from_owner(document, cascade, index, owner);
            (
                RecipeKind::AnonymousTable { owner },
                facts.box_facts,
                Some(facts),
            )
        }
        RecipeKind::AnonymousCell { owner } => {
            let BoxKey::AnonymousCell(slot) = key else {
                panic!("anonymous cell identity")
            };
            let owner = document
                .table_objects
                .cells
                .get(slot)
                .map_or(owner, |cell| cell.owner);
            let facts = TableFacts::from_owner(document, cascade, index, owner);
            (
                RecipeKind::AnonymousCell { owner },
                facts.box_facts,
                Some(facts),
            )
        }
        RecipeKind::Contents | RecipeKind::Spanner => {
            let BoxKey::Source(source) = key else {
                panic!("source wrapper identity")
            };
            let cv = &cascade.computed[source];
            // A lifted spanner wrapper is a block, or an inline box around
            // blocks, and the recipe is the same for both.
            let unchanged = if old_kind == RecipeKind::Contents {
                cv.display == DisplayValue::Contents
            } else {
                matches!(cv.display, DisplayValue::Block | DisplayValue::Inline)
            };
            assert!(
                unchanged,
                "changed wrapper identity requires source bridge: {:?}",
                cv.display
            );
            let direction = crate::layout::bridge::direction_for_axes(
                cv,
                crate::layout::bridge::uses_vertical_axes(cascade, source),
            );
            let mut facts = BoxFacts::for_node(&document.nodes[index], direction);
            if old_kind == RecipeKind::Contents {
                facts.metadata = crate::layout::style_views::metadata::node_metadata(
                    &document.nodes[index],
                    raikiri_layout_style::StyleFlags::EMPTY,
                );
            }
            (old_kind, facts, None)
        }
        RecipeKind::NeutralSource => {
            assert!(matches!(key, BoxKey::Source(_)));
            assert_ne!(
                document.nodes[index].kind(),
                raikiri_traits::NodeKind::Element,
                "changed source identity requires source bridge"
            );
            let facts = BoxFacts {
                direction: LayoutDirection::Ltr,
                metadata: crate::layout::style_views::metadata::node_metadata(
                    &document.nodes[index],
                    raikiri_layout_style::StyleFlags::EMPTY,
                ),
            };
            (old_kind, facts, None)
        }
    };
    let direction = match facts.direction {
        LayoutDirection::Ltr => Direction::Ltr,
        LayoutDirection::Rtl => Direction::Rtl,
    };
    let same_table = table_facts.is_none_or(|facts| {
        values.border_collapse == facts.border_collapse
            && values.border_spacing == facts.border_spacing
            && values.caption_side == facts.caption_side
    });
    let same_dimensions = match kind {
        RecipeKind::InitialContainingBlock { width, height } => {
            values.width == L::Px(width) && values.height == L::Px(height)
        }
        _ => true,
    };
    if old_kind == kind
        && values.direction == direction
        && same_table
        && same_dimensions
        && record.metadata.flags == facts.metadata.flags
        && record.metadata.used_writing_mode == facts.metadata.used_writing_mode
        && record.metadata.claims == facts.metadata.claims
    {
        return None;
    }
    // Allocate a replacement only after current explicit inputs differ.
    Some(match kind {
        RecipeKind::InitialContainingBlock { width, height } => {
            initial_containing_block(width, height, facts)
        }
        RecipeKind::AnonymousTable { .. } => anonymous_table(table_facts.unwrap()),
        RecipeKind::AnonymousCell { .. } => anonymous_cell(table_facts.unwrap()),
        RecipeKind::Contents => contents(facts),
        RecipeKind::Spanner => spanner(facts),
        RecipeKind::NeutralSource => neutral_source(facts),
    })
}

#[cfg(test)]
mod tests;
