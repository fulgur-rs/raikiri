//! CSS parse / cascade 段階の terminal error (Phase B decoupling で移設)。
//!
//! Ownership moved from `raikiri-traits::error` to `raikiri-style` as part of
//! the Stylo-pattern decoupling — raikiri-style now owns its cascade error
//! taxonomy directly, and raikiri-traits re-exports the identity back so
//! `RenderError::Cascade(CascadeError)` at the umbrella surface stays stable.
//!
//! **Stylo/blitz と同じ責務境界**: CSS spec 準拠で invalid rule / value は
//! silently drop され error にならない。cssparser / selectors 固有の error
//! 型は raikiri-style 内部に閉じ込め、この enum は raikiri-style が明示的
//! に fail-hard を選択した場合の signal のみ露出する。
//!
//! 現状 `Internal` variant のみ populate。CSS 実装詳細 (property /
//! value / source location 等) を trait layer に漏らさない。必要になった
//! 時点で `#[non_exhaustive]` の恩恵で追加する。
#[non_exhaustive]
#[derive(Debug)]
pub enum CascadeError {
    /// raikiri-style 内で回復不能な内部 error が発生した (bug 相当、または
    /// 明示的な strict mode で許容外の入力を受けた)。詳細メッセージは
    /// raikiri-style 内部で log + message として構成される。
    Internal {
        /// 人間可読な失敗詳細 (raikiri-style 内部で構成)。
        message: String,
    },
}

impl std::fmt::Display for CascadeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Internal { message } => write!(f, "CSS cascade internal error: {message}"),
        }
    }
}

impl std::error::Error for CascadeError {}
