//! Measure stylesheet-list reads while only unrelated DOM size grows.
//! Build before timing and pin both revisions to the same CPU.

use std::any::Any;
use std::time::Instant;

use raikiri_dom::Document;
use raikiri_js::runtime::{BoxGeometry, DocumentHost, DomRuntime, HostError};

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
        Err(HostError(
            "fragment parsing is not used by this probe".into(),
        ))
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

fn main() {
    for mode in ["length", "item", "missing"] {
        for n in [1000, 2000, 4000, 8000] {
            let mut document = Document::new();
            let html = document.append_element(Some(0), "html", None::<&str>);
            let head = document.append_element(Some(html), "head", None::<&str>);
            document.append_element(Some(head), "style", None::<&str>);
            let body = document.append_element(Some(html), "body", None::<&str>);
            for _ in 0..n {
                document.append_element(Some(body), "div", None::<&str>);
            }
            document.mark_in_document_flags();
            let mut runtime = DomRuntime::new(Host(document)).unwrap();
            assert!(runtime.evaluate("var list=document.styleSheets;var first=list.item(0);list.length===1 && first!==null").unwrap().to_boolean());
            let expression = match mode {
                "length" => "list.length===1",
                "item" => "list.item(0)===first",
                _ => "list.item(1)===null",
            };
            let source = format!(
                "(()=>{{for(let i=0;i<{n};++i)if(!({expression}))return false;return true;}})()"
            );
            let mut samples = Vec::new();
            for _ in 0..5 {
                let start = Instant::now();
                let result = runtime.evaluate(&source).unwrap();
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
                assert!(result.to_boolean());
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "stylesheet list mode={mode} unrelated_nodes={n} queries={n} sheets=1 median_ms={:.6} samples={samples:?}",
                samples[2]
            );
            drop(runtime);
            boa_gc::force_collect();
        }
    }
}
