//! Disposable M0 feasibility spikes (nzv.5-.11).
//!
//! Deleted or moved to examples/ at M1 start per design doc M0.
//! Each module is one spike; see the crate nzv.<N> beads issue for details.
#![allow(dead_code, unused_imports, unused_variables, missing_docs)]

pub mod anyrender_byte_identical; // nzv.7
pub mod cssparser_at_rules; // nzv.9
pub mod paintscene_adapter;
pub mod parley_send_sync; // nzv.5
pub mod selectors_cssparser_compat; // nzv.10
pub mod selectors_standalone; // nzv.8
pub mod taffy_layout_modes; // nzv.6 // nzv.11
