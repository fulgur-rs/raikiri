//! Current CSSOM sources, separate from the original DOM child text.

use super::Document;

#[cfg(test)]
thread_local! {
    static RESET_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn take_reset_work() -> usize {
    RESET_WORK.with(|work| work.replace(0))
}

impl Document {
    /// The current CSSOM source override, including an explicitly empty sheet.
    /// `None` means readers should use the element's DOM child text.
    pub fn stylesheet_source(&self, element: usize) -> Option<&str> {
        self.stylesheet_sources.get(&element).map(AsRef::as_ref)
    }

    /// Replace a style element's CSSOM source without changing its DOM nodes.
    /// Only the latest source is retained. Rendering snapshots share it until
    /// either document replaces it; DOM node cloning does not copy this state.
    pub fn set_stylesheet_source(&mut self, element: usize, source: String) -> Result<(), String> {
        if self.get_node(element).and_then(|node| node.tag_name()) != Some("style") {
            return Err(format!(
                "stylesheet target index {element} is not a style element"
            ));
        }
        self.stylesheet_sources.insert(element, source.into());
        self.invalidate_layout_cache();
        Ok(())
    }

    pub(super) fn reset_stylesheet_source(&mut self, element: usize) {
        #[cfg(test)]
        RESET_WORK.with(|work| work.set(work.get() + 1));
        if self.stylesheet_sources.remove(&element).is_some() {
            self.invalidate_layout_cache();
        }
    }

    pub(super) fn reset_stylesheet_sources_in_subtree(&mut self, root: usize) {
        if self.stylesheet_sources.is_empty() {
            return;
        }
        self.reset_stylesheet_source(root);
        // Walk only the moved subtree. New leaves take constant work even
        // when unrelated sheets elsewhere have CSSOM edits.
        let mut pending = self.nodes[root].children.clone();
        while let Some(node) = pending.pop() {
            if self.stylesheet_sources.is_empty() {
                break;
            }
            self.reset_stylesheet_source(node);
            pending.extend(self.nodes[node].children.iter().copied());
        }
    }

    pub(super) fn stylesheet_has_css_type(&self, element: usize) -> bool {
        self.element_attribute(element, "type")
            .is_none_or(|value| value.is_empty() || value.eq_ignore_ascii_case("text/css"))
    }

    pub(super) fn reset_stylesheet_if_type_changed(&mut self, element: usize, had_css_type: bool) {
        if self.stylesheet_has_css_type(element) != had_css_type {
            self.reset_stylesheet_source(element);
        }
    }
}
