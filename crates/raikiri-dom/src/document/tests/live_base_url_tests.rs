use super::*;

fn url(text: &str) -> url::Url {
    url::Url::parse(text).unwrap()
}

#[test]
fn live_base_url_follows_the_first_html_base_with_an_href() {
    let page = url("file:///wpt/css/page.html");
    let mut doc = Document::new();
    let root = doc.root_index();
    // Without a `<base>`, the document URL is the base.
    assert_eq!(doc.live_base_url(Some(&page)), Some(page.clone()));

    // A non-HTML `<base>` and an HTML one without `href` do not count.
    let svg_base = doc.append_element(Some(root), "base", Style::default(), None::<&str>);
    doc.set_element_namespace(svg_base, Some("http://www.w3.org/2000/svg".into()));
    doc.set_element_attribute(svg_base, "href", "svg/").unwrap();
    doc.append_element(Some(root), "base", Style::default(), None::<&str>);
    assert_eq!(doc.live_base_url(Some(&page)), Some(page.clone()));

    // The first HTML `<base href>` in tree order wins, resolved against the
    // document URL; later ones are ignored.
    let first = doc.append_element(Some(root), "base", Style::default(), None::<&str>);
    doc.set_element_attribute(first, "href", "assets/").unwrap();
    let second = doc.append_element(Some(root), "base", Style::default(), None::<&str>);
    doc.set_element_attribute(second, "href", "other/").unwrap();
    assert_eq!(
        doc.live_base_url(Some(&page)),
        Some(url("file:///wpt/css/assets/"))
    );

    // Changing or removing it is seen on the next call.
    doc.set_element_attribute(first, "href", "/img/").unwrap();
    assert_eq!(doc.live_base_url(Some(&page)), Some(url("file:///img/")));
    doc.remove_element_attribute(first, "href").unwrap();
    assert_eq!(
        doc.live_base_url(Some(&page)),
        Some(url("file:///wpt/css/other/"))
    );
}

#[test]
fn live_base_url_refuses_unusable_hrefs_like_the_parser() {
    let page = url("file:///wpt/page.html");
    let mut doc = Document::new();
    let root = doc.root_index();
    let base = doc.append_element(Some(root), "base", Style::default(), None::<&str>);
    for href in ["data:text/plain,x", "javascript:void(0)", "http://["] {
        doc.set_element_attribute(base, "href", href).unwrap();
        assert_eq!(doc.live_base_url(Some(&page)), Some(page.clone()), "{href}");
    }
    // An absolute `href` needs no document URL; a relative one gives none.
    doc.set_element_attribute(base, "href", "https://example.test/a/")
        .unwrap();
    assert_eq!(
        doc.live_base_url(None),
        Some(url("https://example.test/a/"))
    );
    doc.set_element_attribute(base, "href", "a/").unwrap();
    assert_eq!(doc.live_base_url(None), None);
}
