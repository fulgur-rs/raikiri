use super::*;

#[test]
fn default_is_all_false() {
    let settings = DevtoolSettings::default();
    assert!(!settings.show_layout);
    assert!(!settings.highlight_hover);
}

#[test]
fn toggle_show_layout_flips_twice() {
    let mut settings = DevtoolSettings::default();
    settings.toggle_show_layout();
    assert!(settings.show_layout);
    settings.toggle_show_layout();
    assert!(!settings.show_layout);
}

#[test]
fn toggle_highlight_hover_flips_twice() {
    let mut settings = DevtoolSettings::default();
    settings.toggle_highlight_hover();
    assert!(settings.highlight_hover);
    settings.toggle_highlight_hover();
    assert!(!settings.highlight_hover);
}

#[test]
fn toggles_are_independent() {
    let mut settings = DevtoolSettings::default();
    settings.toggle_show_layout();
    assert!(settings.show_layout);
    assert!(!settings.highlight_hover);
    settings.toggle_highlight_hover();
    assert!(settings.show_layout);
    assert!(settings.highlight_hover);
}
