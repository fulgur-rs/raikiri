//! [`ComputedStyles`]: the computed values of every node, with equal results
//! held once.

use std::ops::{Index, IndexMut};

use triomphe::Arc;

use crate::computed::ComputedValues;

/// The computed values of the nodes of a document, indexed by node, as
/// [`super::CascadeResult::computed`] holds them.
///
/// It reads like a slice of [`ComputedValues`]. Nodes whose results the
/// cascade copied from a sibling or a cousin share one allocation instead
/// of holding a copy each: repeated structures such as table cells, list
/// items and paragraphs have thousands of nodes but only a few distinct
/// values. Writing through [`IndexMut`], [`Self::get_mut`] or
/// [`Self::iter_mut`] gives the written node its own copy first, so a write
/// never shows through another node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComputedStyles {
    values: Vec<Arc<ComputedValues>>,
}

impl ComputedStyles {
    /// No values.
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of nodes.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether there are no nodes.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The values of node `index`, or `None` when it is out of range.
    pub fn get(&self, index: usize) -> Option<&ComputedValues> {
        self.values.get(index).map(|values| &**values)
    }

    /// The values of node `index` to change, or `None` when it is out of
    /// range. The node stops sharing them with other nodes.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut ComputedValues> {
        self.values.get_mut(index).map(Arc::make_mut)
    }

    /// The values of the last node.
    pub fn last(&self) -> Option<&ComputedValues> {
        self.values.last().map(|values| &**values)
    }

    /// The values of every node, in node order.
    pub fn iter(
        &self,
    ) -> impl DoubleEndedIterator<Item = &ComputedValues> + ExactSizeIterator + Clone {
        self.values.iter().map(|values| &**values)
    }

    /// The values of every node to change, in node order. Every node stops
    /// sharing its values, so this costs a copy per node.
    pub fn iter_mut(
        &mut self,
    ) -> impl DoubleEndedIterator<Item = &mut ComputedValues> + ExactSizeIterator {
        self.values.iter_mut().map(Arc::make_mut)
    }

    /// Replaces the values of node `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn set(&mut self, index: usize, values: ComputedValues) {
        self.values[index] = Arc::new(values);
    }

    /// Appends the values of one more node.
    pub fn push(&mut self, values: ComputedValues) {
        self.values.push(Arc::new(values));
    }

    /// Grows to `len` nodes, all the new ones sharing `values`, or keeps
    /// the first `len` nodes when there are more.
    pub fn resize(&mut self, len: usize, values: ComputedValues) {
        if len > self.values.len() {
            self.values.resize(len, Arc::new(values));
        } else {
            self.values.truncate(len);
        }
    }

    /// Keeps the first `len` nodes and drops the rest.
    pub fn truncate(&mut self, len: usize) {
        self.values.truncate(len);
    }

    /// Room for `additional` more nodes, or `Err` when the allocator refuses
    /// it.
    pub(crate) fn try_reserve_exact(
        &mut self,
        additional: usize,
    ) -> Result<(), std::collections::TryReserveError> {
        self.values.try_reserve_exact(additional)
    }

    /// The shared values of node `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub(crate) fn shared(&self, index: usize) -> &Arc<ComputedValues> {
        &self.values[index]
    }

    /// Appends one more node that holds `values`, shared with whatever
    /// holds them already.
    pub(crate) fn push_shared(&mut self, values: Arc<ComputedValues>) {
        self.values.push(values);
    }

    /// Replaces the values of node `index` with `values`, shared with
    /// whatever holds them already.
    pub(crate) fn set_shared(&mut self, index: usize, values: Arc<ComputedValues>) {
        self.values[index] = values;
    }

    /// Grows to `len` nodes, each new one holding `values`.
    pub(crate) fn resize_shared(&mut self, len: usize, values: Arc<ComputedValues>) {
        self.values.resize(len, values);
    }

    /// The values of node `old_of_new[i]` at each index `i`, shared with
    /// this.
    ///
    /// # Panics
    ///
    /// Panics if an index of `old_of_new` is out of range.
    pub(crate) fn renumber(&self, old_of_new: &[usize]) -> Self {
        Self {
            values: old_of_new
                .iter()
                .map(|&old| Arc::clone(&self.values[old]))
                .collect(),
        }
    }

    /// The address of the node buffer, to tell whether it was reallocated.
    #[cfg(test)]
    pub(crate) fn as_ptr(&self) -> *const Arc<ComputedValues> {
        self.values.as_ptr()
    }

    /// Whether nodes `a` and `b` hold one shared allocation.
    #[cfg(test)]
    pub(crate) fn shares(&self, a: usize, b: usize) -> bool {
        Arc::ptr_eq(&self.values[a], &self.values[b])
    }
}

impl Index<usize> for ComputedStyles {
    type Output = ComputedValues;

    fn index(&self, index: usize) -> &ComputedValues {
        &self.values[index]
    }
}

impl IndexMut<usize> for ComputedStyles {
    /// The node stops sharing its values with other nodes.
    fn index_mut(&mut self, index: usize) -> &mut ComputedValues {
        Arc::make_mut(&mut self.values[index])
    }
}

impl From<Vec<ComputedValues>> for ComputedStyles {
    fn from(values: Vec<ComputedValues>) -> Self {
        values.into_iter().collect()
    }
}

impl FromIterator<ComputedValues> for ComputedStyles {
    fn from_iter<I: IntoIterator<Item = ComputedValues>>(iter: I) -> Self {
        Self {
            values: iter.into_iter().map(Arc::new).collect(),
        }
    }
}

impl Extend<ComputedValues> for ComputedStyles {
    fn extend<I: IntoIterator<Item = ComputedValues>>(&mut self, iter: I) {
        self.values.extend(iter.into_iter().map(Arc::new));
    }
}

impl<'a> IntoIterator for &'a ComputedStyles {
    type Item = &'a ComputedValues;
    type IntoIter = std::iter::Map<
        std::slice::Iter<'a, Arc<ComputedValues>>,
        fn(&'a Arc<ComputedValues>) -> &'a ComputedValues,
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.values.iter().map(|values| &**values)
    }
}

impl<'a> IntoIterator for &'a mut ComputedStyles {
    type Item = &'a mut ComputedValues;
    type IntoIter = std::iter::Map<
        std::slice::IterMut<'a, Arc<ComputedValues>>,
        fn(&'a mut Arc<ComputedValues>) -> &'a mut ComputedValues,
    >;

    /// Every node stops sharing its values; see [`ComputedStyles::iter_mut`].
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter_mut().map(Arc::make_mut)
    }
}
