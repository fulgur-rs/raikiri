//! Public parsing exercises the normal, non-test library configuration.

use raikiri_style::property::{GridTemplateAreasValue, PropertyValue};
use raikiri_style::{Origin, RuleTree};

#[test]
fn public_stylesheets_preserve_named_areas_after_index_construction() {
    let names = (0..16)
        .rev()
        .map(|i| format!("area{i}"))
        .collect::<Vec<_>>();
    let row = names.join(" ");
    for property in ["grid-template-areas", "grid-template"] {
        let css = format!("div{{{property}:\"{row}\" \"{row}\";}}");
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&css, Origin::Author);
        assert_eq!(tree.style_rules().len(), 1);
        let areas = tree.style_rules()[0]
            .declarations()
            .iter()
            .find_map(|declaration| match declaration.value() {
                PropertyValue::GridTemplateAreas(GridTemplateAreasValue::Areas(areas)) => {
                    Some(areas)
                }
                _ => None,
            })
            .expect("named-area longhand must survive public stylesheet parsing");
        assert_eq!((areas.row_count, areas.column_count), (2, 16));
        assert_eq!(areas.areas.len(), names.len());
        for (column, (area, name)) in areas.areas.iter().zip(&names).enumerate() {
            assert_eq!(area.name.as_str(), name);
            assert_eq!((area.row_start, area.row_end), (1, 3));
            assert_eq!(
                (area.column_start, area.column_end),
                (column as u32 + 1, column as u32 + 2)
            );
        }
    }
}
