//! A private HTML parser sink whose live handles follow the retained source nodes.

use super::super::*;
use super::RowCapture;
use html5ever::tree_builder::TreeBuilder;
use std::ops::Deref;

type Handle = Rc<Cell<usize>>;

pub(super) struct CompactSink(pub(super) RaikiriTreeSink);

impl Deref for CompactSink {
    type Target = RaikiriTreeSink;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

fn flatten(child: NodeOrText<Handle>) -> NodeOrText<usize> {
    match child {
        NodeOrText::AppendNode(node) => NodeOrText::AppendNode(node.get()),
        NodeOrText::AppendText(text) => NodeOrText::AppendText(text),
    }
}

impl TreeSink for CompactSink {
    type Handle = Handle;
    type Output = UncascadedDocument;
    type ElemName<'a> = Ref<'a, QualName>;

    fn finish(self) -> Self::Output {
        if let Some(state) = self.row_capture.borrow().as_ref() {
            compact(&self, state, Vec::new());
        }
        self.0.finish()
    }
    fn parse_error(&self, msg: Cow<'static, str>) {
        self.0.parse_error(msg);
    }
    fn get_document(&self) -> Handle {
        Rc::new(Cell::new(self.0.get_document()))
    }
    fn elem_name<'a>(&'a self, node: &'a Handle) -> Self::ElemName<'a> {
        Ref::map(self.qual_names.borrow(), |names| &names[&node.get()])
    }
    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> Handle {
        Rc::new(Cell::new(self.0.create_element(name, attrs, flags)))
    }
    fn create_comment(&self, text: StrTendril) -> Handle {
        Rc::new(Cell::new(self.0.create_comment(text)))
    }
    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Handle {
        Rc::new(Cell::new(self.0.create_pi(target, data)))
    }
    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        self.0.append(&parent.get(), flatten(child));
    }
    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        prev: &Handle,
        child: NodeOrText<Handle>,
    ) {
        self.0
            .append_based_on_parent_node(&element.get(), &prev.get(), flatten(child));
    }
    fn append_doctype_to_document(&self, name: StrTendril, public: StrTendril, system: StrTendril) {
        self.0.append_doctype_to_document(name, public, system);
    }
    fn pop(&self, node: &Handle) {
        self.0.pop(&node.get());
    }
    fn get_template_contents(&self, node: &Handle) -> Handle {
        Rc::new(Cell::new(self.0.get_template_contents(&node.get())))
    }
    fn same_node(&self, a: &Handle, b: &Handle) -> bool {
        a.get() == b.get()
    }
    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.0.set_quirks_mode(mode);
    }
    fn append_before_sibling(&self, sibling: &Handle, child: NodeOrText<Handle>) {
        self.0.append_before_sibling(&sibling.get(), flatten(child));
    }
    fn add_attrs_if_missing(&self, node: &Handle, attrs: Vec<Attribute>) {
        self.0.add_attrs_if_missing(&node.get(), attrs);
    }
    fn remove_from_parent(&self, node: &Handle) {
        self.0.remove_from_parent(&node.get());
    }
    fn reparent_children(&self, node: &Handle, parent: &Handle) {
        self.0.reparent_children(&node.get(), &parent.get());
    }
    fn is_mathml_annotation_xml_integration_point(&self, node: &Handle) -> bool {
        self.0
            .is_mathml_annotation_xml_integration_point(&node.get())
    }
}

#[derive(Default)]
struct Live(RefCell<Vec<Handle>>);
impl Tracer for Live {
    type Handle = Handle;
    fn trace_handle(&self, handle: &Handle) {
        self.0.borrow_mut().push(handle.clone());
    }
}

pub(super) fn reclaim(builder: &TreeBuilder<Handle, CompactSink>, state: &RefCell<RowCapture>) {
    if !matches!(&*state.borrow(), RowCapture::Recording(store) if store.closed.len() >= 64) {
        return;
    }
    let live = Live::default();
    builder.trace_handles(&live);
    compact(&builder.sink, state, live.0.into_inner());
}

fn compact(sink: &CompactSink, state: &RefCell<RowCapture>, handles: Vec<Handle>) {
    let mut capture = state.borrow_mut();
    let RowCapture::Recording(store) = &mut *capture else {
        return;
    };
    if store.closed.is_empty() {
        return;
    }
    sink.flush_text();
    let mut dom = sink.document.borrow_mut();
    let mut retained = vec![true; dom.node_count()];
    for row in store.closed.drain(..) {
        for node in dom.preorder(row) {
            retained[node] = false;
        }
    }
    // Inter-row HTML whitespace is outside the cell records. Leaving a text
    // node per line would make each later compaction recopy the whole prefix.
    for (id, keep) in retained.iter_mut().enumerate() {
        if *keep
            && dom
                .get_node(id)
                .and_then(|n| n.text_content())
                .is_some_and(|text| {
                    text.bytes()
                        .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x0c))
                })
            && dom.parent_of(id).and_then(|p| dom.get_node(p)?.tag_name()) == Some("tbody")
        {
            *keep = false;
        }
    }
    let mut keys = Vec::new();
    let mut index = vec![None; retained.len()];
    for (id, keep) in retained.into_iter().enumerate() {
        if keep {
            let node = dom.get_node(id).unwrap();
            if node.template_contents().is_some()
                || dom.parent_of(id).is_some_and(|p| index[p].is_none())
            {
                *capture = RowCapture::Unsupported;
                return;
            }
            index[id] = Some(keys.len());
            keys.push(id);
        }
    }

    let Some(new_ids) = handles
        .iter()
        .map(|h| index.get(h.get()).copied().flatten())
        .collect::<Option<Vec<_>>>()
    else {
        *capture = RowCapture::Unsupported;
        return;
    };
    store.retained_source_ids = keys.iter().map(|id| store.source_id(*id)).collect();
    store.removed_nodes += dom.node_count() - keys.len();
    *dom = dom.extract_nodes(&keys);
    for (handle, id) in handles.into_iter().zip(new_ids) {
        handle.set(id);
    }
    fn remap<V>(map: &RefCell<FxHashMap<usize, V>>, index: &[Option<usize>]) {
        let mut values = map.borrow_mut();
        *values = values
            .drain()
            .filter_map(|(old, v)| index.get(old).copied().flatten().map(|id| (id, v)))
            .collect();
    }
    remap(&sink.qual_names, &index);
    sink.qual_names.borrow_mut().insert(
        OVERFLOW_HANDLE,
        QualName::new(None, ns!(html), "div".into()),
    );
    remap(&sink.attributes, &index);
    remap(&sink.attribute_names, &index);
}
