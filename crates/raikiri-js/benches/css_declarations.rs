//! Public inline CSS declaration reads: cold validation and warmed enumeration.
//! Run cargo bench -p raikiri-js --features css-declaration-diagnostics --bench css_declarations.

// cov:ignore: standalone benchmark is not run by workspace coverage.
mod measurement {
    use raikiri_dom::Document;
    use raikiri_js::css_declaration_diagnostics as diagnostics;
    use raikiri_js::runtime::{BoxGeometry, DocumentHost, DomRuntime, HostError};
    use std::{any::Any, hint::black_box, time::Instant};

    struct Host(Document);
    impl DocumentHost for Host {
        fn document(&self) -> &Document {
            &self.0
        }
        fn document_mut(&mut self) -> &mut Document {
            &mut self.0
        }
        fn flush(&mut self) -> Result<(), HostError> {
            Ok(())
        }
        fn box_geometry(&mut self, _: usize) -> Result<Option<BoxGeometry>, HostError> {
            Ok(None)
        }
        fn computed_value(&mut self, _: usize, _: &str) -> Result<Option<String>, HostError> {
            Ok(None)
        }
        fn parse_fragment(&mut self, _: &str, _: &str, _: &str) -> Result<Document, HostError> {
            Err(HostError("unexpected fragment parse".into()))
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
        fn into_any(self: Box<Self>) -> Box<dyn Any> {
            self
        }
    }

    fn prepare(text: &str, warm: bool) -> DomRuntime {
        let mut document = Document::new();
        document.append_element(Some(0), "html", Some(text));
        document.mark_in_document_flags();
        let mut runtime = DomRuntime::new(Host(document)).unwrap();
        runtime
            .evaluate("globalThis.s=document.documentElement.style;")
            .unwrap();
        if warm {
            runtime.evaluate("s.length").unwrap();
        }
        runtime
    }

    fn measure(n: usize, distinct: bool, enumerate: bool, warm: bool) {
        let text = (0..n)
            .map(|i| format!("--p{:06}:0;", if distinct { i } else { 0 }))
            .collect::<String>();
        let script = if enumerate {
            "(()=>{const n=s.length;let total=0;for(let i=0;i<n;i++){total+=s.item(i).length;}return total;})()"
        } else {
            "s.length"
        };
        let expected = if enumerate {
            n * 9
        } else if distinct {
            n
        } else {
            1
        };
        let mut counted = prepare(&text, warm);
        diagnostics::begin();
        let got = counted.evaluate(script).unwrap();
        let work = diagnostics::finish();
        assert_eq!(got.as_number(), Some(expected as f64));
        drop(counted);

        let mut times = [0.0; 3];
        // Every timed sample has a fresh runtime. Warmed reads explicitly prime
        // the declaration cache outside the clock; input/setup/drop are outside.
        for sample in 0..4 {
            let mut runtime = prepare(&text, warm);
            let start = Instant::now();
            let got = runtime.evaluate(script).unwrap();
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(got.as_number(), Some(expected as f64));
            assert_eq!(diagnostics::finish(), Default::default());
            if sample > 0 {
                times[sample - 1] = elapsed;
            }
            black_box(runtime);
        }
        let mut sorted = times;
        sorted.sort_by(f64::total_cmp);
        println!(
            "n={n},distinct={distinct},enumerate={enumerate},warm={warm},input_bytes={},validation_calls={},raw_declarations={},linear_name_comparisons={},index_lookups={},index_removals={},index_insertions={},cache_hits={},cache_misses={},max_cached_declarations={},max_cached_source_bytes={},median_ms={:.6},samples_ms={:.6};{:.6};{:.6}",
            text.len(),
            work.validation_calls,
            work.raw_declarations,
            work.linear_name_comparisons,
            work.index_lookups,
            work.index_removals,
            work.index_insertions,
            work.cache_hits,
            work.cache_misses,
            work.max_cached_declarations,
            work.max_cached_source_bytes,
            sorted[1],
            times[0],
            times[1],
            times[2]
        );
    }

    pub fn run() {
        for n in [128, 256, 512, 1024] {
            for warm in [false, true] {
                measure(n, false, false, warm);
                measure(n, true, false, warm);
                measure(n, true, true, warm);
            }
        }
    }
}

// cov:ignore: standalone benchmark entry point.
fn main() {
    measurement::run();
}
