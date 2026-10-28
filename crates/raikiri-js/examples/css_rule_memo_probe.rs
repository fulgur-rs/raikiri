//! Measure immutable CSSRuleList reads in a warmed runtime.
//! Build before timing and pin the binary to the same CPU for each revision.

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
    for mode in ["length", "item_selector", "css_text"] {
        for n in [250, 500, 1000, 2000] {
            let mut document = Document::new();
            let html = document.append_element(Some(0), "html", None::<&str>);
            let head = document.append_element(Some(html), "head", None::<&str>);
            let style = document.append_element(Some(head), "style", None::<&str>);
            let text: String = (0..n).map(|i| format!(".c{i}{{color:red}}\n")).collect();
            document.append_text(style, text);
            document.append_element(Some(html), "body", None::<&str>);
            document.mark_in_document_flags();
            let mut runtime = DomRuntime::new(Host(document)).unwrap();
            assert!(runtime.evaluate(&format!(
                "var rules=document.styleSheets[0].cssRules;var held=[];for(var i=0;i<{n};++i)held.push(rules.item(i));rules.length==={n}"
            )).unwrap().to_boolean());
            let expression = match mode {
                "length" => format!(
                    "(()=>{{let total=0;for(let i=0;i<{n};++i)total+=rules.length;return total==={n}*{n};}})()"
                ),
                "item_selector" => format!(
                    "(()=>{{for(let i=0;i<{n};++i){{let r=rules.item(i);if(r!==held[i]||r.selectorText!=='.c'+i)return false;}}return true;}})()"
                ),
                _ => format!(
                    "(()=>{{for(let i=0;i<{n};++i){{let text=held[i].cssText;if(!text.includes('.c'+i)||!text.includes('color:red'))return false;}}return true;}})()"
                ),
            };
            let mut samples = Vec::new();
            for _ in 0..5 {
                let start = Instant::now();
                let result = runtime.evaluate(&expression).unwrap();
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
                assert!(result.to_boolean());
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "css rules mode={mode} n={n} reads={n} median_ms={:.6} samples={samples:?}",
                samples[2]
            );
            drop(runtime);
            boa_gc::force_collect();
        }
    }
}
