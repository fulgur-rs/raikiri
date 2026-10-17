use raikiri_dom::{Document, NodeKind};

#[derive(serde::Serialize)]
pub(crate) struct PageOutput {
    pub results: serde_json::Value,
    pub dom_attribute: String,
    pub mutation_count: usize,
    pub job_callbacks: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Interface {
    Node,
    Element,
}

pub(crate) enum NativeValue {
    Number(u16),
    Node(Option<usize>),
    String(Option<String>),
    Boolean(bool),
    Undefined(()),
}

pub(crate) struct DomError(pub String);

pub(crate) struct Dom {
    pub document: Document,
    pub element: usize,
    pub mutations: usize,
}

impl Dom {
    pub fn new() -> Self {
        let mut document = Document::new();
        let element = document
            .create_detached_element("div")
            .expect("valid fixture tag");
        document.attach_child(document.root_index(), element);
        Self {
            document,
            element,
            mutations: 0,
        }
    }

    pub fn supports(&self, index: usize, interface: Interface) -> bool {
        self.document
            .get_node(index)
            .is_some_and(|node| interface == Interface::Node || node.kind() == NodeKind::Element)
    }

    pub fn interface(&self, index: usize) -> Interface {
        if self.supports(index, Interface::Element) {
            Interface::Element
        } else {
            Interface::Node
        }
    }

    pub fn node_type(&mut self, index: usize) -> Result<u16, DomError> {
        Ok(
            match self
                .document
                .get_node(index)
                .expect("checked receiver")
                .kind()
            {
                NodeKind::Element => 1,
                NodeKind::Document => 9,
                _ => unreachable!("prototype exposes only document and element fixtures"),
            },
        )
    }

    pub fn parent_node(&mut self, index: usize) -> Result<Option<usize>, DomError> {
        Ok(self.document.parent_of(index))
    }

    pub fn is_same_node(&mut self, index: usize, other: Option<usize>) -> Result<bool, DomError> {
        Ok(other == Some(index))
    }

    pub fn get_attribute(
        &mut self,
        index: usize,
        name: String,
    ) -> Result<Option<String>, DomError> {
        Ok(self
            .document
            .element_attribute(index, &name)
            .map(str::to_owned))
    }

    pub fn set_attribute(
        &mut self,
        index: usize,
        name: String,
        value: String,
    ) -> Result<(), DomError> {
        self.document
            .set_element_attribute(index, name, value)
            .map_err(DomError)?;
        self.mutations += 1;
        Ok(())
    }

    pub fn remove_attribute(&mut self, index: usize, name: String) -> Result<(), DomError> {
        self.document
            .remove_element_attribute(index, &name)
            .map_err(DomError)?;
        self.mutations += 1;
        Ok(())
    }
}

// Only binding primitives cross this boundary. JS coercion executes before a DOM borrow.
pub(crate) trait BindingCall {
    type Value;
    type Error;
    fn receiver(&mut self, interface: Interface) -> Result<usize, Self::Error>;
    fn require(&mut self, count: usize) -> Result<(), Self::Error>;
    fn dom_string(&mut self, index: usize) -> Result<String, Self::Error>;
    fn nullable_node(&mut self, index: usize) -> Result<Option<usize>, Self::Error>;
    fn with_dom<R>(&mut self, f: impl FnOnce(&mut Dom) -> R) -> R;
    fn native_error(&mut self, error: DomError) -> Self::Error;
    fn output(&mut self, value: NativeValue) -> Result<Self::Value, Self::Error>;
}
