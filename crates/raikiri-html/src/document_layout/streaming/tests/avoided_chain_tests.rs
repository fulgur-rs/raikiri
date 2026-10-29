use super::*;
use std::{cell::RefCell, rc::Rc};

type PageNodes = Vec<(u32, Vec<NodeId>)>;
struct SharedPages(Rc<RefCell<PageNodes>>);
impl PageSink for SharedPages {
    type Output = StreamSummary;
    fn page(&mut self, page: StreamPage<'_>, _: Vec<ConsumerPropertyEvent>) -> std::io::Result<()> {
        self.0.borrow_mut().push((
            page.index(),
            page.page()
                .fragments()
                .map(|fragment| fragment.node())
                .collect(),
        ));
        Ok(())
    }
    fn finish(self, summary: StreamSummary) -> std::io::Result<StreamSummary> {
        Ok(summary)
    }
}

#[test]
fn avoided_chains_hold_early_pages_and_preserve_batch_page_contents() {
    for avoid in ["auto", "avoid", "avoid-page", "avoid-column"] {
        let prefix = format!(
            "<head><style>@page{{size:80px 80px;margin:0}}body{{margin:0}}div{{height:10px;break-before:{avoid}}}div:first-child{{break-before:auto}}</style></head><body>{}<div>x<!--checkpoint",
            "<div>x</div>".repeat(48)
        );
        let suffix = "--></div></body>";
        let expected = batch_pages(&format!("{prefix}{suffix}"));
        assert!(expected.len() > 1);
        let resources = RenderResources::new();
        let pages = Rc::new(RefCell::new(Vec::new()));
        let mut stream = StreamingLayout::new(
            &resources,
            PageDefaults::default(),
            LayoutConfig::default(),
            SharedPages(pages.clone()),
        )
        .checkpoint_bytes(prefix.len());
        stream.feed(prefix.as_bytes()).unwrap();
        let early = pages.borrow().len();
        if avoid == "auto" {
            assert!(early > 0);
        } else {
            assert_eq!(early, 0, "avoid={avoid}");
        }
        stream.feed(suffix.as_bytes()).unwrap();
        let summary = completed(stream.finish().unwrap());
        assert_eq!(summary.page_count as usize, expected.len());
        assert_eq!(*pages.borrow(), expected, "avoid={avoid}");
    }
}
