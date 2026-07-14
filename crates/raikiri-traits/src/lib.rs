//! raikiri-traits — foundation traits and neutral model types.
//!
//! 設計仕様書 §4 に定義された全 trait / 中立モデル型を集約する。実装は持たず、
//! raikiri-html / raikiri-style / raikiri-dom / raikiri-paint / raikiri-net が
//! 参照する共通型層。詳細は `docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md`
//! §4 を参照。

pub mod dom;

pub use dom::{Dom, Element, NodeId, Node, Symbol};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_construct_from_str() {
        let s = Symbol::from("target-1");
        assert_eq!(s.as_str(), "target-1");
    }

    #[test]
    fn nodeid_construct() {
        let n = NodeId::new(42);
        assert_eq!(n.0, 42);
    }
}
