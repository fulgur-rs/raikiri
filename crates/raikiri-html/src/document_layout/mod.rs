//! Owned layout results and borrowed per-page views for drawing consumers.

mod dom_view;
mod fragment;

pub use dom_view::DomView;
pub use fragment::{Fragment, FragmentKind, RepeatKind};

#[cfg(test)]
mod tests;
