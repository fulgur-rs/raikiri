use raikiri_layout_style::StyleMetadata;
use raikiri_style::property::{BorderCollapseValue, CaptionSideValue, Direction, DisplayValue};
use raikiri_style::{ComputedBorderSpacing, ComputedLengthPercentageOrAuto as L, ComputedValues};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecipeKind {
    InitialContainingBlock,
    AnonymousTable,
    AnonymousCell,
    Contents,
    Spanner,
    NeutralSource,
}

#[derive(Clone, Copy)]
pub(crate) struct BoxFacts {
    pub(crate) direction: taffy::Direction,
    pub(crate) metadata: StyleMetadata,
}

impl BoxFacts {
    pub(crate) fn for_node(node: &crate::node::Node, direction: taffy::Direction) -> Self {
        let mut flags = if node.taffy_display() == taffy::Display::Block {
            raikiri_layout_style::StyleFlags::BLOCK
        } else {
            raikiri_layout_style::StyleFlags::EMPTY
        };
        if node.taffy_display() == taffy::Display::None {
            flags = flags.union(raikiri_layout_style::StyleFlags::HIDDEN);
        }
        Self {
            direction,
            metadata: StyleMetadata {
                flags,
                used_writing_mode: node.used_writing_mode,
                claims: None,
            },
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct TableFacts {
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
        Self {
            box_facts: BoxFacts::for_node(&document.nodes[key], direction),
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
                |_, expression, range| {
                    Arc::new(raikiri_layout_style::AlignedCalc::new(expression, range))
                },
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
        taffy::Direction::Ltr => Direction::Ltr,
        taffy::Direction::Rtl => Direction::Rtl,
    };
    values
}

pub(crate) fn initial_containing_block(width: f32, height: f32, facts: BoxFacts) -> Recipe {
    let mut values = values(facts, DisplayValue::Block);
    values.width = L::Px(width);
    values.height = L::Px(height);
    Recipe {
        kind: RecipeKind::InitialContainingBlock,
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
    table(RecipeKind::AnonymousTable, DisplayValue::Table, facts)
}

pub(crate) fn anonymous_cell(facts: TableFacts) -> Recipe {
    table(RecipeKind::AnonymousCell, DisplayValue::TableCell, facts)
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
        taffy::Direction::Ltr => &LTR,
        taffy::Direction::Rtl => &RTL,
    };
    Recipe {
        kind: RecipeKind::NeutralSource,
        values: shared
            .get_or_init(|| Arc::new(values(facts, DisplayValue::Inline)))
            .clone(),
        metadata: facts.metadata,
    }
}

#[cfg(test)]
mod tests;
