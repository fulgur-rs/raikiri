use super::*;

const PSEUDOS: &str = "div::first-letter,p::first-letter{font-size:20px} \
    div::first-line,p::first-line{color:blue}";

fn origins_fixture(sheet: &str, child_style: &str, prefix: bool) -> (Fixture, usize) {
    let mut child = 0;
    let fixture = sheet_fixture(sheet, "", |doc, root| {
        if prefix {
            doc.append_text(root, "Before");
        }
        child = doc.append_element(Some(root), "p", Some(child_style));
        doc.append_text(child, "A");
    });
    (fixture, child)
}

fn origins(fixture: &Fixture, child: usize, ancestors: bool) -> (Vec<usize>, Vec<usize>) {
    typographic_origins_in(
        &fixture.doc,
        &fixture.cascade,
        child,
        &PredecessorCache::default(),
        ancestors,
        BoxTree::for_cascade(&fixture.doc, &fixture.cascade),
    )
}

#[test]
fn disabling_ancestor_origins_keeps_only_the_current_blocks_pseudos() {
    let (fixture, child) = origins_fixture(PSEUDOS, "display:block", false);
    assert_eq!(origins(&fixture, child, false), (vec![child], vec![child]));
    assert_eq!(
        origins(&fixture, child, true),
        (vec![fixture.root, child], vec![fixture.root, child])
    );
}

#[test]
fn atomic_and_float_boundaries_do_not_take_pseudos_from_above() {
    let (atomic, child) = origins_fixture(PSEUDOS, "display:inline-block", false);
    assert_eq!(origins(&atomic, child, true), (vec![child], vec![child]));
    let (floating, child) = origins_fixture(PSEUDOS, "display:block;float:left", false);
    assert_eq!(origins(&floating, child, true), (vec![child], vec![child]));
}

#[test]
fn a_line_making_parent_before_stops_origins_before_the_parent() {
    let sheet = format!("{PSEUDOS} div::before{{content:'Before'}}");
    let (fixture, child) = origins_fixture(&sheet, "display:block", false);
    assert_eq!(origins(&fixture, child, true), (vec![child], vec![child]));
}

#[test]
fn a_preceding_sibling_stops_origins_before_the_parent() {
    let (fixture, child) = origins_fixture(PSEUDOS, "display:block", true);
    assert_eq!(origins(&fixture, child, true), (vec![child], vec![child]));
}
