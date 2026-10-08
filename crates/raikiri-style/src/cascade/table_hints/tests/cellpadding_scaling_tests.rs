use std::cell::Cell;

use crate::cascade::cascade;
use crate::resolve::ComputedLengthPercentage;
use crate::ruletree::build_rule_tree;
use crate::style_dom::{StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind};
use crate::test_dom::{TestChildIter, TestDoc, TestElementRef, TestNodeRef};

struct CountedDom {
    doc: TestDoc,
    reads: Cell<usize>,
}

struct CountedNode<'a> {
    inner: TestNodeRef<'a>,
    reads: &'a Cell<usize>,
}

struct CountedElement<'a> {
    inner: TestElementRef<'a>,
    reads: &'a Cell<usize>,
}

impl StyleDom for CountedDom {
    type NodeRef<'a> = CountedNode<'a>;
    type ChildIter<'a> = TestChildIter<'a>;

    fn root_id(&self) -> StyleNodeId {
        self.doc.root_id()
    }

    fn node(&self, id: StyleNodeId) -> Option<Self::NodeRef<'_>> {
        self.doc.node(id).map(|inner| CountedNode {
            inner,
            reads: &self.reads,
        })
    }

    fn child_ids(&self, id: StyleNodeId) -> Self::ChildIter<'_> {
        self.doc.child_ids(id)
    }

    fn node_count(&self) -> usize {
        self.doc.node_count()
    }

    fn parent_id(&self, child: StyleNodeId) -> Option<StyleNodeId> {
        self.doc.parent_id(child)
    }
}

impl StyleNode for CountedNode<'_> {
    type Element<'a>
        = CountedElement<'a>
    where
        Self: 'a;

    fn kind(&self) -> StyleNodeKind {
        self.inner.kind()
    }

    fn as_element(&self) -> Option<Self::Element<'_>> {
        self.inner.as_element().map(|inner| CountedElement {
            inner,
            reads: self.reads,
        })
    }

    fn text_content(&self) -> Option<&str> {
        self.inner.text_content()
    }
}

impl StyleElement for CountedElement<'_> {
    fn tag_name(&self) -> &str {
        self.inner.tag_name()
    }

    fn attr(&self, local: &str) -> Option<&str> {
        if local == "cellpadding" {
            self.reads.set(self.reads.get() + 1);
        }
        self.inner.attr(local)
    }
}

#[test]
fn long_cellpadding_is_read_once_per_table_independent_of_cell_count() {
    for attribute in [
        "9".repeat(65_536),
        format!("{}x", " ".repeat(65_536)),
        String::new(),
    ] {
        for count in [1, 64, 1024] {
            let mut doc = TestDoc::new();
            let table =
                doc.push_element_with_attrs(0, "table", None, &[("cellpadding", &attribute)]);
            let row = doc.push_element(table, "tr", None);
            let mut cells = Vec::new();
            for _ in 0..count {
                cells.push(doc.push_element(row, "td", None));
            }
            let mut dom = CountedDom {
                doc,
                reads: Cell::new(0),
            };
            let tree = build_rule_tree(&dom);
            dom.reads.set(0);
            let result = cascade(&dom, &tree).unwrap();
            let expected = if attribute.starts_with('9') {
                u32::MAX as f32
            } else {
                0.0
            };
            for &cell in &cells {
                assert_eq!(
                    result.computed[cell].padding.left,
                    ComputedLengthPercentage::Px(expected)
                );
            }
            assert_eq!(dom.reads.get(), 1, "cell count: {count}");

            // A new cascade must reread mutated DOM attributes rather than retain stale hints.
            dom.doc.nodes[table].attrs[0].1 = "5".into();
            let result = cascade(&dom, &tree).unwrap();
            assert_eq!(dom.reads.get(), 2);
            for &cell in &cells {
                assert_eq!(
                    result.computed[cell].padding.left,
                    ComputedLengthPercentage::Px(5.0)
                );
            }
        }
    }
}
