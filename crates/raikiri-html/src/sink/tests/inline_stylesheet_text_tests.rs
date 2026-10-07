use super::strip_xhtml_cdata_wrapper;

#[test]
fn strips_xhtml_cdata_wrapper_but_keeps_plain_css() {
    assert_eq!(
        strip_xhtml_cdata_wrapper("\n<![CDATA[\nbody { color: red }\n]]>\n"),
        "\nbody { color: red }\n"
    );
    assert_eq!(
        strip_xhtml_cdata_wrapper("body { color: red }"),
        "body { color: red }"
    );
}
