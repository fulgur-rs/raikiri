//! Traced DOM ownership, with weak arena-index lookup and lazy wrappers.

use std::collections::{HashMap, HashSet};
use std::fmt;

use boa_engine::{Finalize, JsObject, Trace};
use boa_gc::{Gc, GcRefCell, WeakGc};
use raikiri_dom::Document;

use super::State;

#[derive(Clone, PartialEq, Eq, Hash, Trace, Finalize)]
pub(super) enum NodeObject {
    Style,
    ComputedStyle,
    ClassList,
    ChildNodes,
    Children,
    CanvasContext,
    StyleSheet,
    RuleList,
}

#[derive(Default, Trace, Finalize)]
pub(super) struct NodeState {
    pub(super) wrapper: Option<JsObject>,
    objects: HashMap<NodeObject, JsObject>,
    listeners: Option<Gc<super::events::ListenerList>>,
    pub(super) css_owner: Option<Gc<super::stylesheet::CssOwner>>,
    parent: Option<Gc<NodeRecord>>,
    children: HashMap<usize, Gc<NodeRecord>>,
    template_contents: Option<Gc<NodeRecord>>,
}

#[derive(Trace, Finalize)]
pub(super) struct NodeRecord {
    #[unsafe_ignore_trace]
    index: usize,
    pub(super) state: GcRefCell<NodeState>,
}

impl fmt::Debug for NodeRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeRecord")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

impl NodeRecord {
    pub(super) fn new(index: usize) -> Gc<Self> {
        Gc::new(Self {
            index,
            state: GcRefCell::default(),
        })
    }
}

pub(super) struct NodeGraph {
    // Only the document is a permanent root. Detached components are kept by
    // script-visible objects and traced edges, never by this lookup table.
    _document: Gc<NodeRecord>,
    records: Vec<Option<WeakGc<NodeRecord>>>,
    #[cfg(test)]
    pub(super) edge_visits: usize,
}

impl NodeGraph {
    pub(super) fn new(document: &mut Document) -> Self {
        let root = document.root_index();
        let record = NodeRecord::new(root);
        let mut graph = Self {
            _document: record.clone(),
            records: vec![None; document.node_count()],
            #[cfg(test)]
            edge_visits: 0,
        };
        graph.records[root] = Some(WeakGc::new(&record));
        graph.restore_subtree(document, root);
        document.enable_tree_change_tracking();
        graph
    }

    fn cached(&self, index: usize) -> Option<Gc<NodeRecord>> {
        self.records.get(index)?.as_ref()?.upgrade()
    }

    pub(super) fn record(&mut self, document: &Document, index: usize) -> Gc<NodeRecord> {
        if let Some(record) = self.cached(index) {
            return record;
        }
        let mut root = index;
        let mut seen = HashSet::new();
        while seen.insert(root) {
            let Some(parent) = document.parent_of(root) else {
                break;
            };
            if self.cached(parent).is_some() {
                break;
            }
            root = parent;
        }
        self.restore_subtree(document, root);
        self.cached(index)
            .expect("the requested node was restored with its component")
    }

    fn restore_subtree(&mut self, document: &Document, root: usize) {
        let mut pending = vec![root];
        let mut seen = HashSet::new();
        let mut records = Vec::new();
        while let Some(index) = pending.pop() {
            if !seen.insert(index) {
                continue;
            }
            let node = document
                .get_node(index)
                .expect("ownership edges name existing arena nodes");
            let record = self.cached(index).unwrap_or_else(|| {
                let record = NodeRecord::new(index);
                self.records[index] = Some(WeakGc::new(&record));
                record
            });
            // Keep every new record rooted until all edges are installed.
            // Gc::new may collect, but no traced cell is borrowed here.
            records.push(record);
            pending.extend(node.children.iter().copied());
            pending.extend(node.template_contents());
        }
        for record in &records {
            self.refresh_edges(document, record);
        }
    }

    fn refresh_edges(&mut self, document: &Document, record: &Gc<NodeRecord>) {
        #[cfg(test)]
        {
            self.edge_visits += 1;
        }
        let parent = document
            .parent_of(record.index)
            .map(|index| self.record(document, index));
        let old_parent = record.state.borrow().parent.clone();
        if old_parent.as_ref().map(|parent| parent.index)
            != parent.as_ref().map(|parent| parent.index)
        {
            if let Some(old_parent) = old_parent {
                old_parent.state.borrow_mut().children.remove(&record.index);
            }
            if let Some(parent) = &parent {
                parent
                    .state
                    .borrow_mut()
                    .children
                    .insert(record.index, record.clone());
            }
            record.state.borrow_mut().parent = parent;
        }
        let contents = document
            .get_node(record.index)
            .and_then(|node| node.template_contents())
            .map(|index| self.record(document, index));
        record.state.borrow_mut().template_contents = contents;
    }

    /// Synchronize ownership edges and report native topology notifications.
    /// New detached arena slots alone do not change existing membership.
    fn synchronize(&mut self, document: &mut Document) -> bool {
        let previous_count = self.records.len();
        let count = document.node_count();
        self.records.resize(count, None);
        let mut changed = document.take_tree_changes();
        let topology_changed = !changed.is_empty();
        changed.extend(previous_count..count);
        // Unlinking an old edge must not collect another changed record
        // before the batch has reconnected it to its final parent.
        let _roots: Vec<_> = changed.iter().filter_map(|&id| self.cached(id)).collect();
        for index in changed {
            if let Some(record) = self.cached(index) {
                self.refresh_edges(document, &record);
            } else if document
                .parent_of(index)
                .is_some_and(|parent| self.cached(parent).is_some())
            {
                self.restore_subtree(document, index);
            }
        }
        topology_changed
    }
}

impl State {
    pub(super) fn listeners(
        &mut self,
        key: Option<usize>,
    ) -> Option<Gc<super::events::ListenerList>> {
        if let Some(index) = key {
            self.node_record(index).state.borrow().listeners.clone()
        } else {
            self.window_listeners.clone()
        }
    }
    pub(super) fn ensure_listeners(
        &mut self,
        key: Option<usize>,
    ) -> Gc<super::events::ListenerList> {
        if let Some(index) = key {
            self.node_record(index)
                .state
                .borrow_mut()
                .listeners
                .get_or_insert_with(|| Gc::new(super::events::ListenerList::default()))
                .clone()
        } else {
            self.window_listeners
                .get_or_insert_with(|| Gc::new(super::events::ListenerList::default()))
                .clone()
        }
    }

    pub(super) fn synchronize_node_graph(&mut self) {
        // Preserve a notification across the empty sync after a read.
        self.image_topology_changed |= self.node_graph.synchronize(self.host.document_mut());
    }

    pub(super) fn node_record(&mut self, index: usize) -> Gc<NodeRecord> {
        self.node_graph.record(self.host.document(), index)
    }

    pub(super) fn node_object(&mut self, index: usize, key: NodeObject) -> Option<JsObject> {
        let record = self.node_record(index);
        record.state.borrow().objects.get(&key).cloned()
    }

    pub(super) fn set_node_object(
        &mut self,
        index: usize,
        key: NodeObject,
        object: JsObject,
    ) -> Option<JsObject> {
        let record = self.node_record(index);
        record.state.borrow_mut().objects.insert(key, object)
    }
}
