#[test]
fn shodo_defaults_that_the_ifc_path_relies_on_are_stable() {
    // The exact version pin in the workspace manifest is the real contract;
    // this fails loudly when a bump changes the bounded defaults.
    let limits = shodo::limits::Limits::default();
    assert_eq!(limits.max_warnings, Some(1024));
    assert!(limits.max_text_bytes.is_some());
}
