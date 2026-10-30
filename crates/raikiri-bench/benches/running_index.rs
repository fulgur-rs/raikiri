//! Public RunningIndex construction after layout/projection, with work counts.
//! Run RAYON_NUM_THREADS=1 cargo bench -p raikiri-bench --bench running_index.

// cov:ignore: standalone benchmark is not run by workspace coverage.
mod measurement {
    use raikiri_dom::fonts::{BundledFace, build_bundled_font_collection};
    use raikiri_dom::{
        Document, PageContentInsets, PageMargins, RunningIndex, begin_running_index_diagnostics,
        finish_running_index_diagnostics, layout_pages,
    };
    use raikiri_style::property::StringFetchMode;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;
    use std::{hint::black_box, time::Instant};

    fn verify(index: &RunningIndex, count: usize, selected: Option<usize>) {
        assert_eq!(index.nodes().count(), count);
        assert_eq!(
            index
                .select("h", StringFetchMode::Last, 0)
                .map(|node| node.0 as usize),
            selected
        );
    }

    fn measure(n: usize, mode: &str, install: &impl Fn(&mut Document)) {
        let mut document = Document::new();
        install(&mut document);
        let html = document.append_element(
            Some(0),
            "html",
            Some("display:block;font-family:Ahem;font-size:10px"),
        );
        let body = document.append_element(Some(html), "body", Some("display:block"));
        let mut count = 0;
        let mut selected = None;
        for i in 0..n {
            let active = match mode {
                "all" => true,
                "one" => i == 0,
                "two" => i == 0 || i == n - 1,
                _ => false,
            };
            let id = document.append_element(
                Some(body),
                "div",
                Some(if active {
                    "display:block;position:running(h)"
                } else {
                    "display:none"
                }),
            );
            if active {
                count += 1;
                selected = Some(id);
            }
        }
        let paragraph = document.append_element(Some(body), "p", Some("display:block"));
        document.append_text(paragraph, "a");
        document.mark_in_document_flags();
        let cascade = cascade(&document, &build_rule_tree(&document)).unwrap();
        let page = PageBox::new();
        let slices = layout_pages(&mut document, &cascade, page).unwrap();
        assert_eq!(slices.len(), 1);
        let geometries = slices
            .iter()
            .map(|_| (page, PageMargins::default(), PageContentInsets::default()))
            .collect::<Vec<_>>();
        document
            .project_pages(&cascade, page, &slices, &geometries)
            .unwrap();
        assert!(document.page_fragments(0).count() < 10);

        begin_running_index_diagnostics();
        let counted = RunningIndex::build(&document, &cascade, [(&document, 0)]);
        let work = finish_running_index_diagnostics();
        verify(&counted, count, selected);
        drop(counted);
        let mut times = [0.0; 5];
        for sample in 0..6 {
            let start = Instant::now();
            let index = RunningIndex::build(&document, &cascade, [(&document, 0)]);
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            verify(&index, count, selected);
            assert_eq!(finish_running_index_diagnostics(), Default::default());
            if sample > 0 {
                times[sample - 1] = elapsed;
            }
            black_box(index);
        }
        let mut sorted = times;
        sorted.sort_by(f64::total_cmp);
        println!(
            "n={n},mode={mode},placements={count},next_visits={},previous_visits={},own_visits={},prepared_nodes={},prepared_slots={},indexed_probes={},index_builds={},scratch_bytes={},median_ms={:.6},samples_ms={:.6};{:.6};{:.6};{:.6};{:.6}",
            work.next_candidate_visits,
            work.previous_candidate_visits,
            work.own_candidate_visits,
            work.prepared_nodes,
            work.prepared_slots,
            work.indexed_probes,
            work.index_builds,
            work.scratch_bytes,
            sorted[2],
            times[0],
            times[1],
            times[2],
            times[3],
            times[4]
        );
    }

    pub fn run() {
        let fonts = build_bundled_font_collection(
            vec![BundledFace {
                family: "Ahem".into(),
                bytes: include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf")
                    .to_vec(),
            }],
            false,
        )
        .unwrap();
        let install = |document: &mut Document| document.set_font_collection(fonts.clone());
        for n in [1000, 2000, 4000, 8000] {
            for mode in ["none", "one", "two", "all"] {
                measure(n, mode, &install);
            }
        }
    }
}

// cov:ignore: standalone benchmark entry point.
fn main() {
    measurement::run();
}
