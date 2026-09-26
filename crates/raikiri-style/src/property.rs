//! CSS property value 型と per-property parser。
//!
//! 現サポート property の canonical 一覧は `parse_value` の match arm を参照
//! (該 arm を single source of truth として扱う)。認識できない property name /
//! invalid value は `parse_value` が `None` を返す (spec 準拠の silent drop、
//! caller である rule.rs で declaration ごと drop)。
//!
//! `parse_value` は rule.rs の `DeclParser::parse_value` から呼ばれる。

mod types;
pub use types::*;

mod parse;
pub use parse::*;

mod names;
pub use names::*;

mod calc_serialize;

mod serialize;
pub use serialize::*;

#[cfg(test)]
mod tests;
