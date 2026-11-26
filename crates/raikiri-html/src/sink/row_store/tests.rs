use super::*;

fn encoded_cells(texts: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for text in texts {
        bytes.extend_from_slice(&(text.len() as u64).to_le_bytes());
        bytes.extend_from_slice(text.as_bytes());
    }
    bytes
}

fn cell_payloads(mut record: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    while !record.is_empty() {
        let len = usize::try_from(u64::from_le_bytes(record[16..24].try_into().unwrap())).unwrap();
        bytes.extend_from_slice(&record[16..24 + len]);
        record = &record[24 + len..];
    }
    bytes
}

#[test]
fn reads_exact_ranges_in_any_order_after_appending() {
    let mut store = RowStore::new().unwrap();
    store.append(b"<tr>one</tr>").unwrap();
    store.append("<tr>二</tr>".as_bytes()).unwrap();
    assert_eq!(store.read_rows(1..2).unwrap(), "<tr>二</tr>".as_bytes());
    store.append(b"<tr>3</tr>").unwrap();
    assert_eq!(store.read_rows(0..1).unwrap(), b"<tr>one</tr>");
    assert_eq!(
        store.read_rows(1..3).unwrap(),
        "<tr>二</tr><tr>3</tr>".as_bytes()
    );
    assert!(store.read_rows(3..3).unwrap().is_empty());
    assert_eq!(
        store.read_rows(1..4).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn stores_are_document_local() {
    let mut first = RowStore::new().unwrap();
    let mut second = RowStore::new().unwrap();
    first.append(b"a").unwrap();
    second.append(b"b").unwrap();
    drop(first);
    assert_eq!(second.read_rows(0..1).unwrap(), b"b");
}

#[test]
fn parser_records_implicitly_closed_rows_from_chunked_input() {
    use crate::sink::RaikiriTreeSink;
    use html5ever::tendril::TendrilSink;

    let mut parser = html5ever::parse_document(RaikiriTreeSink::new(None), Default::default());
    let capture = parser.tokenizer.sink.sink.capture_table_rows().unwrap();
    for chunk in [
        "<table><tbody><tr><td>a &amp;",
        " b<tr><td>&lt;二",
        "</table>",
    ] {
        parser.process(chunk.into());
    }
    parser.finish();
    let mut capture = capture.borrow_mut();
    let RowCapture::Recording(store) = &mut *capture else {
        panic!("expected recorded rows")
    };
    assert_eq!(
        cell_payloads(&store.read_rows(0..2).unwrap()),
        encoded_cells(&["a & b", "<二"])
    );
}

#[test]
fn unsupported_late_cell_leaves_the_parsed_document_intact() {
    use crate::sink::RaikiriTreeSink;
    use html5ever::tendril::TendrilSink;

    let parser = html5ever::parse_document(RaikiriTreeSink::new(None), Default::default());
    let capture = parser.tokenizer.sink.sink.capture_table_rows().unwrap();
    let parsed = parser.one("<table><tr><td>a<tr><td rowspan=2>b</table>");
    assert!(matches!(*capture.borrow(), RowCapture::Unsupported));
    assert_eq!(
        parsed
            .dom
            .preorder(parsed.dom.root_index())
            .filter(|&id| parsed.dom.get_node(id).unwrap().tag_name() == Some("tr"))
            .count(),
        2
    );
}

#[test]
fn finish_reports_a_buffered_disk_write_failure() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let reader = File::open(file.path()).unwrap();
    let mut store = RowStore {
        file: BufWriter::new(reader),
        ends: vec![0],
        at_end: true,
        closed: Vec::new(),
        retained_source_ids: Vec::new(),
        removed_nodes: 0,
        row_source_ids: Vec::new(),
    };
    store.append(b"buffered").unwrap();
    assert!(RowCapture::Recording(store).finish().is_err());
    let error = RowCapture::Failed(io::Error::other("row write"))
        .finish()
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "row write");
    assert!(RowCapture::Unsupported.finish().unwrap().is_none());
}

#[test]
fn late_fallback_replays_consumed_bytes_once_before_the_unread_suffix() {
    let html = "<table><tr><td>一&amp;二<tr><td rowspan=2>b</tr></table><p id=end>after";
    let bytes = html.as_bytes();
    let mut probe = TableInput::new(&raikiri_traits::RenderLimits::default()).unwrap();
    let consumed = bytes
        .iter()
        .position(|b| !probe.feed(std::slice::from_ref(b)).unwrap())
        .unwrap()
        + 1;
    assert!(consumed < bytes.len());
    let mut replayed = Vec::new();
    probe
        .into_replay()
        .unwrap()
        .chain(&bytes[consumed..])
        .read_to_end(&mut replayed)
        .unwrap();
    assert_eq!(replayed, bytes);
    let doc = crate::parse(replayed.as_slice(), &Default::default()).unwrap();
    assert!(
        doc.dom.preorder(doc.dom.root_index()).any(|id| doc
            .dom
            .get_node(id)
            .unwrap()
            .attribute("id")
            == Some("end"))
    );
}

#[test]
fn eof_closes_last_row_and_retains_input_for_late_style_checks() {
    let html = b"<table><tr><td>one<tr><td>two";
    let mut probe = TableInput::new(&Default::default()).unwrap();
    assert!(probe.feed(html).unwrap());
    let (_, rows, mut original) = probe.finish().unwrap();
    let mut replay = Vec::new();
    original.read_to_end(&mut replay).unwrap();
    assert_eq!(replay, html);
    assert_eq!(
        cell_payloads(&rows.unwrap().read_rows(0..2).unwrap()),
        encoded_cells(&["one", "two"])
    );
}

#[test]
fn probe_enforces_input_limits_and_rejects_truncated_utf8() {
    use raikiri_traits::{LimitKind, ParseError, RenderLimits};
    let mut limits = RenderLimits::default();
    limits.max_input_bytes = Some(3);
    let mut probe = TableInput::new(&limits).unwrap();
    assert!(probe.feed(b"abc").unwrap());
    assert!(matches!(
        probe.feed(b"d"),
        Err(ParseError::LimitExceeded {
            kind: LimitKind::InputBytes,
            actual: 4,
            ..
        })
    ));
    let mut probe = TableInput::new(&Default::default()).unwrap();
    assert!(probe.feed(&[0xe3]).unwrap());
    assert!(matches!(probe.finish(), Err(ParseError::Encoding { .. })));
}

#[test]
fn closed_rows_leave_the_parser_arena_while_the_next_row_is_open() {
    for total in [4_000, 40_000] {
        let mut probe = TableInput::new(&Default::default()).unwrap();
        probe.feed(b"<table><tbody>").unwrap();
        let mut peak_nodes = 0;
        for i in 0..total {
            assert!(
                probe
                    .feed(format!("<tr><td>{i}<td>item<td>2</tr>\n").as_bytes())
                    .unwrap()
            );
            let arena = probe.parser.parser().tokenizer.sink.sink.document.borrow();
            peak_nodes = peak_nodes.max(arena.node_count());
        }
        assert!(peak_nodes < 520, "{total} rows kept {peak_nodes} nodes");
        let (_, rows, _) = probe.finish().unwrap();
        let last = total - 1;
        assert_eq!(
            cell_payloads(&rows.unwrap().read_rows(last..total).unwrap()),
            encoded_cells(&[&last.to_string(), "item", "2"])
        );
    }
}

#[test]
fn reclaim_preserves_live_handles_and_enforces_total_node_budget() {
    let mut probe = TableInput::new(&Default::default()).unwrap();
    assert!(
        probe
            .feed(b"<html lang=ja><body id=main><table><tbody>")
            .unwrap()
    );
    for _ in 0..257 {
        assert!(probe.feed(b"<tr><td>x").unwrap());
    }
    assert!(probe.feed(b"</tbody></table><p id=end>after").unwrap());
    let (shell, rows, _) = probe.finish().unwrap();
    let tags: Vec<_> = shell
        .dom
        .preorder(shell.dom.root_index())
        .filter_map(|id| shell.dom.get_node(id).unwrap().tag_name())
        .collect();
    assert!(!tags.contains(&"tr"));
    assert!(tags.contains(&"tbody"));
    assert!(
        shell.dom.preorder(shell.dom.root_index()).any(|id| shell
            .dom
            .get_node(id)
            .unwrap()
            .attribute("id")
            == Some("end"))
    );
    assert_eq!(
        cell_payloads(&rows.unwrap().read_rows(256..257).unwrap()),
        encoded_cells(&["x"])
    );

    let mut limits = raikiri_traits::RenderLimits::default();
    limits.max_dom_nodes = Some(550);
    let mut probe = TableInput::new(&limits).unwrap();
    probe.feed(b"<table>").unwrap();
    let result = (0..1000).try_for_each(|_| probe.feed(b"<tr><td>x").map(|_| ()));
    assert!(matches!(
        result,
        Err(raikiri_traits::ParseError::LimitExceeded {
            kind: raikiri_traits::LimitKind::DomNodes,
            limit: 550,
            ..
        })
    ));
}

#[test]
fn replay_window_preserves_header_styles_and_only_selected_body_rows() {
    let html = b"<style>td{height:23px}</style><table><thead><tr><th>Header</thead><tbody><tr><td>a<tr><td>b &amp; c<tr><td></table>";
    let mut probe = TableInput::new(&Default::default()).unwrap();
    probe.feed(html).unwrap();
    let (shell, rows, _) = probe.finish().unwrap();
    let shell = crate::parse::finish_document(shell, &Default::default()).unwrap();
    let mut windows = TableWindows::new(shell, rows.unwrap()).unwrap();
    assert_eq!(windows.len(), 3);
    let window = windows.document(1..3).unwrap();
    let text: Vec<_> = window
        .dom
        .preorder(0)
        .filter_map(|id| window.dom.get_node(id).unwrap().text_content())
        .collect();
    assert!(text.contains(&"Header") && text.contains(&"b & c") && !text.contains(&"a"));
    assert_eq!(window.stylesheet_sources, windows.shell.stylesheet_sources);
    assert_eq!(
        window
            .dom
            .preorder(0)
            .filter(|&id| window.dom.get_node(id).unwrap().tag_name() == Some("td"))
            .count(),
        2
    );
    let first = windows.document(0..1).unwrap();
    assert!(
        first
            .dom
            .preorder(0)
            .any(|id| first.dom.get_node(id).unwrap().text_content() == Some("a"))
    );
}

#[test]
fn two_bodies_cannot_share_a_flat_row_store() {
    let mut probe = TableInput::new(&Default::default()).unwrap();
    probe
        .feed(b"<table><tbody><tr><td>a<tbody><tr><td>b</table>")
        .unwrap();
    let (windows, mut replay) = probe.finish_windows(&Default::default()).unwrap();
    assert!(windows.is_none());
    let mut original = String::new();
    replay.read_to_string(&mut original).unwrap();
    assert_eq!(original, "<table><tbody><tr><td>a<tbody><tr><td>b</table>");
}

#[test]
fn restored_window_has_normal_fragment_geometry() {
    let prefix = "<style>td,th{height:23px;border:1px solid}table{width:240px}</style><table><thead><tr><th>Header</thead><tbody>";
    let selected = "<tr><td>b &amp; c<td>2<tr><td>last<td>";
    let source = format!("{prefix}<tr><td>omitted<td>x{selected}</table>");
    let expected = format!("{prefix}{selected}</table>");
    let options = Default::default();
    let limits = Default::default();
    let mut input = TableInput::new(&limits).unwrap();
    assert!(input.feed(source.as_bytes()).unwrap());
    let (windows, _) = input.finish_windows(&options).unwrap();
    let restored = windows.unwrap().document(1..3).unwrap();
    let restored =
        crate::document_parse::assemble_document(restored, &options, &limits, None, None).unwrap();
    let normal = crate::parse_html_with_limits(expected.as_bytes(), &options, limits).unwrap();
    let geometry = |doc: &crate::HtmlDocument| {
        let crate::LayoutStatus::Completed(result) = crate::layout(
            doc,
            Default::default(),
            Default::default(),
            crate::LayoutOptions::new(),
        )
        .unwrap() else {
            panic!("layout aborted")
        };
        result
            .pages()
            .map(|page| page.fragments().map(|f| f.rect()).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    };
    let expected = geometry(&normal);
    assert!(expected.iter().flatten().count() >= 6);
    assert_eq!(geometry(&restored), expected);
}

#[test]
fn measured_windows_keep_the_column_boundary_set_by_an_absent_row() {
    let html = b"<style>table{width:360px}td{border:1px solid}</style><table><tr><td>short<td>1<tr><td>x<td>MMMMMMMMMMMMMMMM<td>third column<tr><td>end</table>";
    let options = Default::default();
    let limits = Default::default();
    let mut input = TableInput::new(&limits).unwrap();
    input.feed(html).unwrap();
    let mut windows = input.finish_windows(&options).unwrap().0.unwrap();
    let boundaries = |doc: &crate::HtmlDocument| {
        let crate::LayoutStatus::Completed(result) = crate::layout(
            doc,
            Default::default(),
            Default::default(),
            crate::LayoutOptions::new(),
        )
        .unwrap() else {
            panic!("layout aborted")
        };
        let mut cells = Vec::new();
        for page in result.pages() {
            for fragment in page.fragments() {
                if doc
                    .dom()
                    .get_node(fragment.node().0 as usize)
                    .and_then(|node| node.tag_name())
                    == Some("td")
                {
                    let rect = fragment.rect();
                    cells.push((rect.x, rect.width));
                }
            }
        }
        cells
    };
    let assemble = |shell| {
        crate::document_parse::assemble_document(shell, &options, &limits, None, None).unwrap()
    };
    let all = crate::parse_html_with_limits(&html[..], &options, limits.clone()).unwrap();
    let expected = boundaries(&all);
    assert_eq!(expected.len(), 6);
    assert_ne!(
        boundaries(&assemble(windows.document(0..1).unwrap())),
        expected[..2]
    );
    let resources = crate::RenderResources::new();
    let table = windows.table();
    for window_size in 1..=3 {
        windows.columns = None;
        windows
            .measure_columns(std::num::NonZeroUsize::new(window_size).unwrap(), |shell| {
                let document = assemble(shell);
                crate::render::measure_table_columns(
                    &document,
                    Default::default(),
                    &Default::default(),
                    &resources,
                    &[],
                    table,
                )
            })
            .unwrap();
        for (row, cells) in [0..2, 2..5, 5..6].into_iter().enumerate() {
            let shell = windows.document(row..row + 1).unwrap();
            assert_eq!(boundaries(&assemble(shell)), expected[cells]);
        }
    }
}

#[test]
fn page_windows_preserve_body_row_coordinates_after_each_break() {
    use std::fmt::Write;
    let mut html = String::from(
        "<!doctype html><style>@page{size:300px 180px;margin:10px;@bottom-center{content:counter(page) '/' counter(pages)}}body{margin:0}table{width:100%;border-spacing:0}td,th{height:21.25px;padding:0}tr{break-inside:avoid}</style><table><thead><tr><th>Header</thead><tbody>",
    );
    for row in 0..53 {
        write!(html, "<tr><td>{row}").unwrap();
    }
    html.push_str("</tbody></table>");
    let options = Default::default();
    let limits = Default::default();
    let mut input = TableInput::new(&limits).unwrap();
    input.feed(html.as_bytes()).unwrap();
    let mut windows = input.finish_windows(&options).unwrap().0.unwrap();
    let pages = |doc: &crate::HtmlDocument| {
        let crate::LayoutStatus::Completed(layout) = crate::layout(
            doc,
            Default::default(),
            Default::default(),
            crate::LayoutOptions::new(),
        )
        .unwrap() else {
            panic!("layout aborted")
        };
        let rows = layout
            .pages()
            .map(|page| {
                page.fragments()
                    .filter_map(|fragment| {
                        let id = fragment.node().0 as usize;
                        let node = doc.dom().get_node(id)?;
                        if node.tag_name() != Some("td") {
                            return None;
                        }
                        Some((
                            doc.dom()
                                .element_text_content(id)?
                                .parse::<usize>()
                                .unwrap(),
                            fragment.rect(),
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        (layout, rows)
    };
    let expected =
        pages(&crate::parse_html_with_limits(html.as_bytes(), &options, limits.clone()).unwrap());
    let (reference_layout, expected) = expected;
    assert!(expected.len() >= 5);
    for lookahead in [14, 23] {
        let assemble =
            |shell| crate::document_parse::assemble_document(shell, &options, &limits, None, None);
        let plan = windows
            .plan_pages(std::num::NonZeroUsize::new(lookahead).unwrap(), |shell| {
                let doc = assemble(shell)?;
                let (layout, _) = pages(&doc);
                Ok((doc, layout))
            })
            .unwrap()
            .unwrap();
        let mut actual = Vec::new();
        let total = plan.iter().map(|window| window.pages.len() as u32).sum();
        for window in plan {
            let doc = assemble(windows.document(window.rows).unwrap()).unwrap();
            let (layout, rows) = pages(&doc);
            for (local, cells) in rows
                .into_iter()
                .enumerate()
                .take(window.pages.end)
                .skip(window.pages.start)
            {
                let index = actual.len() as u32;
                let streamed = crate::StreamPage::from_window(&layout, local, index, total);
                assert_eq!(streamed.index(), index);
                assert_eq!(streamed.page().index(), index);
                assert_eq!(
                    streamed.page().margin_boxes()[0].content,
                    format!("{}/{total}", index + 1)
                );
                assert_eq!(
                    streamed.page().fragments().count(),
                    reference_layout.page(index).unwrap().fragments().count()
                );
                let repeats = |page: crate::Page<'_>| {
                    page.fragments()
                        .filter_map(|fragment| {
                            fragment.repeat().map(|kind| (kind, fragment.rect()))
                        })
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    repeats(streamed.page()),
                    repeats(reference_layout.page(index).unwrap())
                );
                actual.push(cells);
            }
        }
        assert_eq!(actual.len(), expected.len());
        for (number, (page, reference)) in actual.iter().zip(&expected).enumerate() {
            assert_eq!(page, reference, "page {number}, lookahead {lookahead}");
        }
    }
}

#[test]
fn split_row_in_later_window_discards_the_entire_page_plan() {
    let mut html = String::from(
        "<style>@page{size:180px 140px;margin:0}body{margin:0}table{width:120px;border-spacing:0}td{height:22px}tr{break-inside:avoid}</style><table>",
    );
    for _ in 0..40 {
        html.push_str("<tr><td>short");
    }
    html.push_str("<tr><td>");
    html.push_str(&"word ".repeat(400));
    html.push_str("</table>");
    let limits = Default::default();
    let options = Default::default();
    let mut input = TableInput::new(&limits).unwrap();
    input.feed(html.as_bytes()).unwrap();
    let (mut windows, mut original) = input.finish_windows(&options).unwrap();
    let mut calls = 0;
    let mut split_pages = 0;
    let plan = windows
        .as_mut()
        .unwrap()
        .plan_pages(std::num::NonZeroUsize::new(14).unwrap(), |shell| {
            calls += 1;
            let doc =
                crate::document_parse::assemble_document(shell, &options, &limits, None, None)?;
            let crate::LayoutStatus::Completed(result) = crate::layout(
                &doc,
                Default::default(),
                Default::default(),
                crate::LayoutOptions::new(),
            )?
            else {
                panic!("layout aborted")
            };
            if let Some(row) = doc.dom().preorder(0).find(|&id| {
                doc.dom().get_node(id).and_then(|node| node.tag_name()) == Some("tr")
                    && doc
                        .dom()
                        .element_text_content(id)
                        .is_some_and(|text| text.starts_with("word "))
            }) {
                split_pages = result
                    .pages()
                    .filter(|page| {
                        page.fragments()
                            .any(|fragment| fragment.node().0 as usize == row)
                    })
                    .count();
            }
            Ok((doc, result))
        })
        .unwrap();
    assert!(calls > 1);
    assert!(split_pages > 1);
    assert!(plan.is_none());
    let mut replay = String::new();
    original.read_to_string(&mut replay).unwrap();
    assert_eq!(replay, html);
}

#[test]
fn content_outside_replayable_body_rows_keeps_the_original_input() {
    for html in [
        "<p>before<table><tr><td>row</table>",
        "<table><tr><td>row</table>after",
        "<table id=destination><tr><td>row</table>",
        "<table><thead><tr><th><a href='#x'>heading</a></thead><tr><td>row</table>",
        "<table><caption>once</caption><tr><td>row</table>",
        "<table><tr><td>row</table><style>td{color:red}</style>",
        "<table><tr><td>row</table><table><tr><td>second</table>",
    ] {
        let mut parser = TableInput::new(&Default::default()).unwrap();
        parser.feed(html.as_bytes()).unwrap();
        let (windows, mut replay) = parser.finish_windows(&Default::default()).unwrap();
        assert!(windows.is_none(), "{html}");
        let mut original = String::new();
        replay.read_to_string(&mut original).unwrap();
        assert_eq!(original, html);
    }
}

#[test]
fn selectors_that_depend_on_rows_outside_a_window_require_replay() {
    for (selector, expected) in [
        ("table > tbody tr td", true),
        ("tr:nth-child(odd) td", false),
        ("tr + tr", false),
        ("table:has(td)", false),
        ("@media print { tr:last-child { color: blue } } td", false),
    ] {
        let html = format!("<style>{selector} {{color:red}}</style><table><tr><td>x</table>");
        let limits = Default::default();
        let mut probe = TableInput::new(&limits).unwrap();
        probe.feed(html.as_bytes()).unwrap();
        let windows = probe
            .finish_windows(&Default::default())
            .unwrap()
            .0
            .unwrap();
        assert_eq!(
            windows.permits_window_styles(&[], &limits).unwrap(),
            expected,
            "{selector}"
        );
    }
}

#[test]
fn page_context_and_stateful_declarations_require_document_layout() {
    for (css, expected) in [
        (
            "@page {size: A4; margin: 12px; @bottom-center {content:counter(page) '/' counter(pages)}}",
            true,
        ),
        ("td, th {border:1px solid; padding:2px}", true),
        ("body {font:12px/16px Ahem}", false),
        (
            "body {font-family:Ahem;font-size:12px;line-height:16px}",
            true,
        ),
        ("td {border-image-source:url(border.png)}", false),
        ("@page :first {size: A4}", false),
        ("@page :left {margin-left:32px}", false),
        ("@page chapter {size: A4}", false),
        ("@page {counter-increment:page 3}", false),
        ("@page {@top-left {content:string(chapter)}}", false),
        (
            "@page {@bottom-center {content:counter(page); color:red}}",
            false,
        ),
        ("td {counter-increment: row}", false),
        ("table {position:fixed}", false),
        ("tr {page:chapter}", false),
    ] {
        let limits = Default::default();
        let html = format!("<style>{css}</style><table><tr><td>row</table>");
        let mut parser = TableInput::new(&limits).unwrap();
        parser.feed(html.as_bytes()).unwrap();
        let windows = parser
            .finish_windows(&Default::default())
            .unwrap()
            .0
            .unwrap();
        assert_eq!(
            windows.permits_window_styles(&[], &limits).unwrap(),
            expected,
            "{css}"
        );
    }
}

#[test]
fn spooled_row_ids_match_batch_after_repeated_compaction() {
    let html = format!(
        "<table>{}</table>",
        "<tr><td>text</td><td>23</td></tr>\n".repeat(950)
    );
    let options = Default::default();
    let limits = Default::default();
    let mut input = TableInput::new(&limits).unwrap();
    for chunk in html.as_bytes().chunks(1300) {
        input.feed(chunk).unwrap();
    }
    let (windows, _) = input.finish_windows(&options).unwrap();
    let mut windows = windows.unwrap();
    let doc = crate::parse_html_with_limits(html.as_bytes(), &options, limits).unwrap();
    let dom = doc.dom();
    let rows: Vec<_> = dom
        .preorder(dom.root_index())
        .filter(|id| dom.get_node(*id).unwrap().tag_name() == Some("tr"))
        .collect();
    assert_eq!(windows.rows.row_source_ids, rows);
    let window = windows.document(730..755).unwrap();
    for local in window.dom.preorder(window.dom.root_index()) {
        let source = window.dom.source_node_index(local).unwrap();
        assert_eq!(window.dom.local_node_index(source), Some(local));
        let a = window.dom.get_node(local).unwrap();
        let b = dom.get_node(source).unwrap();
        assert_eq!(a.tag_name(), b.tag_name());
        assert_eq!(a.text_content(), b.text_content());
        let parent = window
            .dom
            .parent_of(local)
            .and_then(|p| window.dom.source_node_index(p));
        assert_eq!(parent, dom.parent_of(source));
    }
}

#[test]
fn template_contents_survive_replay_after_row_compaction_is_declined() {
    let mut input = String::from("<!doctype html><template><b>saved</b></template><table>");
    input.push_str(&"<tr><td>x</td></tr>".repeat(80));
    input.push_str("</table>");
    let mut probe = TableInput::new(&Default::default()).unwrap();
    assert!(!probe.feed(input.as_bytes()).unwrap());
    let mut original = String::new();
    probe
        .into_replay()
        .unwrap()
        .read_to_string(&mut original)
        .unwrap();
    assert_eq!(original, input);
    let parsed = crate::parse(original.as_bytes(), &Default::default()).unwrap();
    let dom = &parsed.dom;
    let contents = dom
        .preorder(dom.root_index())
        .find_map(|id| dom.get_node(id)?.template_contents())
        .unwrap();
    assert!(
        dom.preorder(contents)
            .any(|id| dom.get_node(id).unwrap().text_content() == Some("saved"))
    );
    assert_eq!(
        dom.preorder(dom.root_index())
            .filter(|&id| dom.get_node(id).unwrap().tag_name() == Some("tr"))
            .count(),
        80
    );
}

#[test]
fn table_input_reports_io_failure_while_recording_a_closed_row() {
    let mut probe = TableInput::new(&Default::default()).unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    if let RowCapture::Recording(store) = &mut *probe.capture.borrow_mut() {
        store.file = BufWriter::with_capacity(1, File::open(file.path()).unwrap());
    }
    let result = probe.feed(b"<table><tr><td>first</td></tr><tr><td>second</td></tr>");
    assert!(matches!(result, Err(raikiri_traits::ParseError::Io(_))));
}

#[test]
fn truncated_row_storage_reports_an_incomplete_read() {
    let mut store = RowStore::new().unwrap();
    store.append(b"first").unwrap();
    store.append(b"second").unwrap();
    store.file.flush().unwrap();
    store.file.get_ref().set_len(7).unwrap();
    assert_eq!(
        store.read_rows(1..2).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    assert_eq!(store.read_rows(0..1).unwrap(), b"first");
}

#[test]
fn parser_repairs_survive_compaction_of_preceding_rows() {
    for suffix in [
        "misplaced text</table>",
        "<div>fostered</div></table>",
        "</table><b><p>one</b>two</p>",
        "</table><html lang=ja><body id=late><math><annotation-xml encoding='text/html'><p>nested</p></annotation-xml></math>",
    ] {
        let prefix = format!(
            "<!doctype html><table><tbody>{}",
            "<tr><td>x</td></tr>".repeat(80)
        );
        let mut probe = TableInput::new(&Default::default()).unwrap();
        assert!(probe.feed(prefix.as_bytes()).unwrap());
        assert!(probe.feed(suffix.as_bytes()).unwrap());
        let (shell, rows, _) = probe.finish().unwrap();
        assert_eq!(rows.unwrap().row_source_ids.len(), 80);

        let reference =
            crate::parse(format!("{prefix}{suffix}").as_bytes(), &Default::default()).unwrap();
        let retained = |doc: &crate::types::UncascadedDocument| {
            let dom = &doc.dom;
            let mut nodes = Vec::new();
            for id in dom.preorder(dom.root_index()) {
                let mut path = Vec::new();
                let mut next = Some(id);
                while let Some(i) = next {
                    let node = dom.get_node(i).unwrap();
                    path.push(node.tag_name().unwrap_or("").to_owned());
                    next = dom.parent_of(i);
                }
                if path.iter().any(|tag| tag == "tr") {
                    continue;
                }
                let node = dom.get_node(id).unwrap();
                nodes.push((
                    path,
                    node.text_content().map(str::to_owned),
                    node.attribute("id").map(str::to_owned),
                    node.attribute("lang").map(str::to_owned),
                ));
            }
            nodes
        };
        assert_eq!(retained(&shell), retained(&reference), "{suffix}");
    }
}

#[test]
fn page_prefix_rejects_displaced_rows_and_misaligned_continuations() {
    let html = b"<!doctype html><table><tr><td>a<tr><td>b</table>";
    let limits = Default::default();
    let options = Default::default();
    let mut input = TableInput::new(&limits).unwrap();
    input.feed(html).unwrap();
    let mut windows = input.finish_windows(&options).unwrap().0.unwrap();
    let shell = windows.document(0..2).unwrap();
    let doc =
        crate::document_parse::assemble_document(shell, &options, &limits, None, None).unwrap();
    let crate::LayoutStatus::Completed(layout) = crate::layout(
        &doc,
        Default::default(),
        Default::default(),
        crate::LayoutOptions::new(),
    )
    .unwrap() else {
        panic!("layout aborted")
    };
    let row = doc.dom().get_node(windows.group).unwrap().children[0];
    let mut rect = layout
        .page(0)
        .unwrap()
        .fragments()
        .find(|f| f.node().0 as usize == row)
        .unwrap()
        .rect();
    assert!(
        windows
            .page_prefix(0..2, &doc, &layout, 0, 0, &mut vec![rect])
            .is_some()
    );
    rect.y += 0.25;
    assert!(
        windows
            .page_prefix(0..2, &doc, &layout, 0, 0, &mut vec![rect])
            .is_none()
    );
    assert!(
        windows
            .page_prefix(0..2, &doc, &layout, 1, 0, &mut Vec::new())
            .is_none()
    );
    assert!(
        windows
            .page_prefix(0..1, &doc, &layout, 0, 0, &mut Vec::new())
            .is_none()
    );
}

#[test]
fn malformed_stored_cells_return_invalid_data_without_affecting_other_rows() {
    let mut input = TableInput::new(&Default::default()).unwrap();
    input.feed(b"<table><tr><td>a<tr><td>b</table>").unwrap();
    let mut windows = input
        .finish_windows(&Default::default())
        .unwrap()
        .0
        .unwrap();
    let second = windows.rows.ends[1];
    for (offset, replacement) in [(24, &[0xff][..]), (16, &u64::MAX.to_le_bytes()[..])] {
        windows
            .rows
            .file
            .seek(SeekFrom::Start(second + offset))
            .unwrap();
        windows.rows.file.write_all(replacement).unwrap();
        windows.rows.file.flush().unwrap();
        let Err(error) = windows.document(1..2) else {
            panic!("invalid cell was accepted")
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let first = windows.document(0..1).unwrap();
        assert!(
            first.dom.preorder(first.dom.root_index()).any(|id| first
                .dom
                .get_node(id)
                .unwrap()
                .text_content()
                == Some("a"))
        );
    }
    for header_len in [1, 8, 16] {
        windows.rows.ends[2] = second + header_len;
        let Err(error) = windows.document(1..2) else {
            panic!("incomplete header was accepted")
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}

#[test]
fn implicit_eof_nodes_count_towards_the_parser_limit() {
    let mut limits = raikiri_traits::RenderLimits::default();
    limits.max_dom_nodes = Some(1);
    let mut input = TableInput::new(&limits).unwrap();
    assert!(input.feed(b"").unwrap());
    assert!(matches!(
        input.finish(),
        Err(raikiri_traits::ParseError::LimitExceeded {
            kind: raikiri_traits::LimitKind::DomNodes,
            limit: 1,
            ..
        })
    ));
}

#[test]
fn table_style_eligibility_propagates_the_selector_budget() {
    let mut limits = raikiri_traits::RenderLimits::default();
    limits.max_style_selectors = Some(0);
    let mut input = TableInput::new(&limits).unwrap();
    input
        .feed(b"<style>td {color:red}</style><table><tr><td>x</table>")
        .unwrap();
    let windows = input
        .finish_windows(&Default::default())
        .unwrap()
        .0
        .unwrap();
    assert!(matches!(
        windows.permits_window_styles(&[], &limits),
        Err(raikiri_traits::RenderError::LimitExceeded {
            kind: raikiri_traits::LimitKind::StyleSelectors,
            limit: 0,
            ..
        })
    ));
}

#[test]
fn complex_rows_discard_prior_record_counts_and_preserve_source_for_replay() {
    for row in [
        "<tr class=keep><td>b</tr>",
        "<tr><td>a<!--note-->b</td></tr>",
    ] {
        let mut input = TableInput::new(&Default::default()).unwrap();
        let prefix = "<table><tr><td>first</td></tr>";
        assert!(input.feed(prefix.as_bytes()).unwrap());
        assert_eq!(input.recorded_rows(), 1);
        assert!(!input.feed(row.as_bytes()).unwrap());
        assert_eq!(input.recorded_rows(), 0);
        let mut replay = String::new();
        input
            .into_replay()
            .unwrap()
            .read_to_string(&mut replay)
            .unwrap();
        assert_eq!(replay, format!("{prefix}{row}"));
    }
}

#[test]
fn windows_without_a_confirmable_row_boundary_require_replay() {
    for (css, width) in [("", 1), ("tr { display:none }", 8)] {
        let html = format!(
            "<!doctype html><style>{css}</style><table>{}</table>",
            "<tr><td>x</td></tr>".repeat(8)
        );
        let limits = Default::default();
        let options = Default::default();
        let mut input = TableInput::new(&limits).unwrap();
        input.feed(html.as_bytes()).unwrap();
        let (windows, mut source) = input.finish_windows(&options).unwrap();
        let plan = windows
            .unwrap()
            .plan_pages(std::num::NonZeroUsize::new(width).unwrap(), |shell| {
                let doc =
                    crate::document_parse::assemble_document(shell, &options, &limits, None, None)?;
                let crate::LayoutStatus::Completed(layout) = crate::layout(
                    &doc,
                    Default::default(),
                    Default::default(),
                    crate::LayoutOptions::new(),
                )?
                else {
                    panic!("layout aborted")
                };
                Ok((doc, layout))
            })
            .unwrap();
        assert!(plan.is_none());
        let mut original = String::new();
        source.read_to_string(&mut original).unwrap();
        assert_eq!(original, html);
    }
}

#[test]
fn stored_cells_can_be_visited_before_layout_including_empty_and_decoded_text() {
    let mut input = TableInput::new(&Default::default()).unwrap();
    input
        .feed("<table><tr><td>first<tr><td><td>二 &amp; three</table>".as_bytes())
        .unwrap();
    let mut windows = input
        .finish_windows(&Default::default())
        .unwrap()
        .0
        .unwrap();
    let mut observed = Vec::new();
    windows
        .rows
        .visit_rows(1..2, |row_id, mut row| {
            while let Some(cell) = row.next_cell()? {
                observed.push((row_id, cell.source_id, cell.text_id, cell.text.to_owned()));
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(observed.len(), 2);
    let (row, empty, absent_text, empty_value) = &observed[0];
    let (same_row, filled, text_id, value) = &observed[1];
    assert_eq!(row, same_row);
    assert!(row < empty && empty < filled && Some(*filled) < *text_id);
    assert_eq!(*absent_text, None);
    assert!(empty_value.is_empty());
    assert_eq!(value, "二 & three");
}
