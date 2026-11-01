//! Length comparisons retained until their percentage basis is known.

use super::{CalcLengthPercentage, Length};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

mod parse;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq)]
enum Node<L> {
    Leaf(L),
    Sum(Box<[Self]>),
    Scale(Box<Self>, f32),
    Min(Box<[Self]>),
    Max(Box<[Self]>),
    Clamp(Box<[Self; 3]>),
}

impl<L> Node<L> {
    fn map<T>(&self, leaf: &mut impl FnMut(&L) -> Option<Node<T>>) -> Option<Node<T>> {
        let mut children = |nodes: &[Self]| {
            nodes
                .iter()
                .map(|node| node.map(leaf))
                .collect::<Option<Box<[_]>>>()
        };
        Some(match self {
            Self::Leaf(value) => leaf(value)?,
            Self::Sum(nodes) => Node::Sum(children(nodes)?),
            Self::Scale(node, factor) => Node::Scale(Box::new(node.map(leaf)?), *factor),
            Self::Min(nodes) => Node::Min(children(nodes)?),
            Self::Max(nodes) => Node::Max(children(nodes)?),
            Self::Clamp(nodes) => Node::Clamp(Box::new([
                nodes[0].map(leaf)?,
                nodes[1].map(leaf)?,
                nodes[2].map(leaf)?,
            ])),
        })
    }

    fn evaluate(&self, leaf: &impl Fn(&L) -> f32) -> f32 {
        match self {
            Self::Leaf(value) => leaf(value),
            Self::Sum(nodes) => nodes.iter().map(|node| node.evaluate(leaf)).sum(),
            Self::Scale(node, factor) => node.evaluate(leaf) * factor,
            Self::Min(nodes) => nodes
                .iter()
                .map(|node| node.evaluate(leaf))
                .fold(f32::INFINITY, f32::min),
            Self::Max(nodes) => nodes
                .iter()
                .map(|node| node.evaluate(leaf))
                .fold(f32::NEG_INFINITY, f32::max),
            Self::Clamp(nodes) => nodes[0]
                .evaluate(leaf)
                .max(nodes[1].evaluate(leaf).min(nodes[2].evaluate(leaf))),
        }
    }

    fn css(&self, leaf: &impl Fn(&L) -> String) -> String {
        let list = |nodes: &[Self], separator: &str| {
            nodes
                .iter()
                .map(|node| node.css(leaf))
                .collect::<Vec<_>>()
                .join(separator)
        };
        match self {
            Self::Leaf(value) => leaf(value),
            Self::Sum(nodes) => format!("calc({})", list(nodes, " + ")),
            Self::Scale(node, factor) => format!("calc({} * {factor})", node.css(leaf)),
            Self::Min(nodes) => format!("min({})", list(nodes, ", ")),
            Self::Max(nodes) => format!("max({})", list(nodes, ", ")),
            Self::Clamp(nodes) => format!("clamp({})", list(nodes.as_slice(), ", ")),
        }
    }
}

/// A specified length comparison, including relative length leaves.
#[derive(Clone, Debug, PartialEq)]
#[doc(hidden)]
pub struct LengthMath {
    root: Node<Length>,
}

impl LengthMath {
    /// Parse a length expression containing at least one comparison function.
    pub fn parse(input: &mut cssparser::Parser<'_, '_>) -> Option<Arc<Self>> {
        parse::parse(input)
    }

    /// Absolutize each leaf without selecting a comparison's winning branch.
    pub(crate) fn absolutize(
        &self,
        mut leaf: impl FnMut(Length) -> Option<CalcLengthPercentage>,
    ) -> Option<Arc<ComputedLengthMath>> {
        let root = self.root.map(&mut |value| {
            let value = leaf(*value)?;
            (value.px.is_finite() && value.percent.is_finite()).then_some(Node::Leaf(value))
        })?;
        Some(Arc::new(ComputedLengthMath { root }))
    }

    /// Evaluate with the caller's used-length resolver, without allocating a tree.
    /// The caller must absolutize relative units and apply the property's range
    /// and finite-value policy to the whole result after evaluation.
    pub fn resolve_with(&self, leaf: impl Fn(Length) -> f32) -> f32 {
        self.root.evaluate(&|value| leaf(*value))
    }

    /// Lift an already computed expression without parsing CSS again.
    pub fn from_computed(value: &ComputedLengthMath) -> Arc<Self> {
        let root = value
            .root
            .map(&mut |value| {
                Some(if value.percent == 0.0 {
                    Node::Leaf(Length::Px(value.px))
                } else if value.px == 0.0 {
                    Node::Leaf(Length::Percent(value.percent))
                } else {
                    Node::Sum(Box::new([
                        Node::Leaf(Length::Px(value.px)),
                        Node::Leaf(Length::Percent(value.percent)),
                    ]))
                })
            })
            .expect("lifting computed leaves is total");
        Arc::new(Self { root })
    }

    /// Serialize the retained expression without evaluating percentages.
    pub fn to_css(&self) -> String {
        self.root.css(&super::serialize_length)
    }
}

/// An immutable comparison of absolutized lengths and unresolved percentages.
#[derive(Clone, Debug, PartialEq)]
#[doc(hidden)]
pub struct ComputedLengthMath {
    root: Node<CalcLengthPercentage>,
}

// Construction keeps every stored coefficient finite; evaluation may overflow,
// but that result is never stored in the tree. Numeric equality is reflexive.
impl Eq for ComputedLengthMath {}

impl Hash for ComputedLengthMath {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.root.hash_node(state);
    }
}

impl ComputedLengthMath {
    /// Resolve all branches against the same used percentage basis.
    /// Property range checking belongs to the caller, after this evaluation.
    pub fn resolve(&self, basis: f32) -> f32 {
        self.root
            .evaluate(&|value| value.px + basis * value.percent / 100.0)
    }

    /// Whether any branch depends on a percentage, even if it loses at zero.
    pub fn has_percentage(&self) -> bool {
        self.root.has_percentage()
    }

    /// Serialize the expression with absolutized length leaves.
    pub fn to_css(&self) -> String {
        self.root.css(&super::serialize_calc_length_percentage)
    }
}

impl Node<CalcLengthPercentage> {
    fn hash_node<H: Hasher>(&self, state: &mut H) {
        let float = |value: f32, state: &mut H| {
            (if value == 0.0 { 0 } else { value.to_bits() }).hash(state);
        };
        match self {
            Self::Leaf(value) => {
                0u8.hash(state);
                float(value.px, state);
                float(value.percent, state);
            }
            Self::Scale(node, factor) => {
                1u8.hash(state);
                float(*factor, state);
                node.hash_node(state);
            }
            Self::Sum(nodes) | Self::Min(nodes) | Self::Max(nodes) => {
                let tag = match self {
                    Self::Sum(_) => 2u8,
                    Self::Min(_) => 3,
                    _ => 4,
                };
                tag.hash(state);
                nodes.len().hash(state);
                for node in nodes {
                    node.hash_node(state);
                }
            }
            Self::Clamp(nodes) => {
                5u8.hash(state);
                for node in nodes.iter() {
                    node.hash_node(state);
                }
            }
        }
    }

    fn has_percentage(&self) -> bool {
        match self {
            Self::Leaf(value) => value.percent != 0.0,
            Self::Scale(node, _) => node.has_percentage(),
            Self::Sum(nodes) | Self::Min(nodes) | Self::Max(nodes) => {
                nodes.iter().any(Self::has_percentage)
            }
            Self::Clamp(nodes) => nodes.iter().any(Self::has_percentage),
        }
    }
}
