//! Stable cascade-layer identities and context-dependent ordering.
//!
//! CSS Cascading Level 5 sections 6.4.2 and 6.4.3:
//! <https://www.w3.org/TR/css-cascade-5/#layer-order>.

use crate::media::{MediaCondition, MediaContext};
use crate::ruletree::Origin;
use cssparser::{Parser, Token};
use smol_str::SmolStr;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_LAYER_TABLE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LayerId {
    owner: u64,
    index: usize,
}

pub(crate) type LayerName = Vec<SmolStr>;

/// Parse optional block names or a statement's comma-separated name list.
pub(crate) fn parse_layer_names<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<LayerName>, cssparser::ParseError<'i, ()>> {
    if input.is_exhausted() {
        return Ok(Vec::new());
    }
    input.parse_comma_separated(|input| {
        let mut name = vec![layer_ident(input.expect_ident_cloned()?, input)?];
        while let Ok(token) = input.next_including_whitespace() {
            match token {
                Token::Delim('.') => {
                    let ident = match input.next_including_whitespace()?.clone() {
                        Token::Ident(ident) => ident,
                        _ => return Err(input.new_custom_error(())),
                    };
                    if name.len() >= 128 {
                        return Err(input.new_custom_error(()));
                    }
                    name.push(layer_ident(ident, input)?);
                }
                Token::WhiteSpace(_) => {
                    input.expect_exhausted()?;
                    break;
                }
                _ => return Err(input.new_custom_error(())),
            }
        }
        Ok(name)
    })
}

fn layer_ident<'i>(
    ident: cssparser::CowRcStr<'i>,
    input: &Parser<'i, '_>,
) -> Result<SmolStr, cssparser::ParseError<'i, ()>> {
    if ["initial", "inherit", "unset", "revert", "revert-layer"]
        .iter()
        .any(|keyword| ident.eq_ignore_ascii_case(keyword))
    {
        return Err(input.new_custom_error(()));
    }
    Ok(SmolStr::new(ident.as_ref()))
}

struct LayerNode {
    parent: Option<LayerId>,
    origin: usize,
}
struct LayerMention {
    layer: LayerId,
    condition: Option<MediaCondition>,
}

pub(crate) struct LayerTable {
    owner: u64,
    nodes: Vec<LayerNode>,
    named: HashMap<(usize, Option<LayerId>, SmolStr), LayerId>,
    mentions: Vec<LayerMention>,
}

impl Default for LayerTable {
    fn default() -> Self {
        Self {
            owner: NEXT_LAYER_TABLE.fetch_add(1, Ordering::Relaxed),
            nodes: Vec::new(),
            named: HashMap::new(),
            mentions: Vec::new(),
        }
    }
}

impl LayerTable {
    pub(crate) fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// An anonymous block always creates a distinct segment. Named paths
    /// reopen existing layers within the same parent and cascade origin.
    pub(crate) fn declare(
        &mut self,
        parent: Option<LayerId>,
        name: Option<&LayerName>,
        origin: Origin,
        condition: Option<&MediaCondition>,
    ) -> LayerId {
        let origin = match origin {
            Origin::UserAgent => 0,
            Origin::User => 1,
            Origin::AuthorPresentationalHint => 2,
            Origin::Author => 3,
            Origin::Animation => 4,
        };
        let mut parent = parent;
        if let Some(name) = name {
            for segment in name {
                let key = (origin, parent, segment.clone());
                let id = *self.named.entry(key).or_insert_with(|| {
                    let id = LayerId {
                        owner: self.owner,
                        index: self.nodes.len(),
                    };
                    self.nodes.push(LayerNode { parent, origin });
                    id
                });
                self.mentions.push(LayerMention {
                    layer: id,
                    condition: condition.cloned(),
                });
                parent = Some(id);
            }
            // Parsed names always contain at least one identifier.
            if let Some(parent) = parent {
                return parent;
            }
        }
        let id = LayerId {
            owner: self.owner,
            index: self.nodes.len(),
        };
        self.nodes.push(LayerNode { parent, origin });
        self.mentions.push(LayerMention {
            layer: id,
            condition: condition.cloned(),
        });
        id
    }

    /// `None` excludes conditional declarations for compatibility registry views.
    pub(crate) fn order(&self, context: Option<&MediaContext>) -> LayerOrder {
        let mut seen = vec![false; self.nodes.len()];
        let mut children = vec![Vec::new(); self.nodes.len()];
        let mut roots: [Vec<LayerId>; 5] = std::array::from_fn(|_| Vec::new());
        for mention in &self.mentions {
            let active = match (&mention.condition, context) {
                (None, _) => true,
                (Some(condition), Some(context)) => condition.matches(context),
                (Some(_), None) => false,
            };
            if !active || seen[mention.layer.index] {
                continue;
            }
            seen[mention.layer.index] = true;
            let node = &self.nodes[mention.layer.index];
            if let Some(parent) = node.parent {
                children[parent.index].push(mention.layer);
            } else {
                roots[node.origin].push(mention.layer);
            }
        }
        let mut ranks = vec![u32::MAX; self.nodes.len()];
        for roots in roots {
            let mut rank = 0u32;
            let mut stack: Vec<_> = roots.into_iter().rev().map(|id| (id, false)).collect();
            while let Some((id, exiting)) = stack.pop() {
                if exiting {
                    ranks[id.index] = rank;
                    rank = rank.saturating_add(1).min(u32::MAX - 1);
                } else {
                    stack.push((id, true));
                    stack.extend(children[id.index].iter().rev().map(|&child| (child, false)));
                }
            }
        }
        LayerOrder {
            owner: self.owner,
            ranks,
        }
    }
}

pub(crate) struct LayerOrder {
    owner: u64,
    ranks: Vec<u32>,
}
impl LayerOrder {
    pub(crate) fn rank(&self, layer: Option<LayerId>) -> u32 {
        layer
            .filter(|id| id.owner == self.owner)
            .and_then(|id| self.ranks.get(id.index))
            .copied()
            .unwrap_or(u32::MAX)
    }
    pub(crate) fn priority(&self, layer: Option<LayerId>, important: bool) -> u32 {
        let normal = self.rank(layer);
        if important { u32::MAX - normal } else { normal }
    }
}

/// Declaration placement before importance reverses the layer order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LayerPosition {
    pub(crate) attached: bool,
    pub(crate) rank: u32,
}
impl Default for LayerPosition {
    fn default() -> Self {
        Self {
            attached: false,
            rank: u32::MAX,
        }
    }
}
impl LayerPosition {
    pub(crate) fn priority(self, important: bool) -> (bool, u32) {
        (
            self.attached,
            if important {
                u32::MAX - self.rank
            } else {
                self.rank
            },
        )
    }
}
