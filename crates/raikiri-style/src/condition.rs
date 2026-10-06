//! Shared boolean conditions and three-valued evaluation.

/// Kleene's three-valued AND (Media Queries 4 §3.1); `None` is "unknown".
pub(crate) const fn kleene_and(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    match (left, right) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}

pub(crate) const fn kleene_not(value: Option<bool>) -> Option<bool> {
    match value {
        Some(value) => Some(!value),
        None => None,
    }
}

const fn kleene_or(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    kleene_not(kleene_and(kleene_not(left), kleene_not(right)))
}

/// A boolean condition over leaves of type `L`, evaluated with three-valued
/// logic.
///
/// `Unknown` stands for a `<general-enclosed>` term or a feature this engine
/// does not evaluate; MQ4 §3.2 gives both the value "unknown".
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Condition<L> {
    Not(Box<Self>),
    And(Vec<Self>),
    Or(Vec<Self>),
    Leaf(L),
    Unknown,
}

impl<L> Condition<L> {
    pub(crate) fn all(head: Self, rest: Vec<Self>) -> Self {
        if rest.is_empty() {
            head
        } else {
            Self::And(std::iter::once(head).chain(rest).collect())
        }
    }

    pub(crate) fn any(head: Self, rest: Vec<Self>) -> Self {
        if rest.is_empty() {
            head
        } else {
            Self::Or(std::iter::once(head).chain(rest).collect())
        }
    }

    pub(crate) fn eval(&self, leaf: &impl Fn(&L) -> Option<bool>) -> Option<bool> {
        match self {
            Self::Not(inner) => kleene_not(inner.eval(leaf)),
            // `None` is a third truth value here, not an early exit.
            Self::And(terms) => terms
                .iter()
                .map(|term| term.eval(leaf))
                .reduce(kleene_and)
                .unwrap_or(Some(true)),
            Self::Or(terms) => terms
                .iter()
                .map(|term| term.eval(leaf))
                .reduce(kleene_or)
                .unwrap_or(Some(false)),
            Self::Leaf(value) => leaf(value),
            Self::Unknown => None,
        }
    }

    /// Every value this condition can take, assuming each leaf can be either
    /// true or false.
    pub(crate) fn outcomes(&self) -> Outcomes {
        match self {
            Self::Not(inner) => inner.outcomes().map(kleene_not),
            Self::And(terms) => terms.iter().fold(Outcomes::TRUE, |acc, term| {
                acc.zip(term.outcomes(), kleene_and)
            }),
            Self::Or(terms) => terms.iter().fold(Outcomes::FALSE, |acc, term| {
                acc.zip(term.outcomes(), kleene_or)
            }),
            Self::Leaf(_) => Outcomes::EITHER,
            Self::Unknown => Outcomes::UNKNOWN,
        }
    }
}

/// A set of three-valued results, used to drop queries that can never match.
///
/// [`crate::media::parse_media_prelude`] returns `None` for a list that matches in no
/// context, and callers skip such `@media` blocks entirely, so a query that is
/// always false or unknown has to be recognised while parsing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Outcomes(u8);

impl Outcomes {
    const VALUES: [Option<bool>; 3] = [Some(true), Some(false), None];
    pub(crate) const TRUE: Self = Self(1);
    pub(crate) const FALSE: Self = Self(2);
    const UNKNOWN: Self = Self(4);
    pub(crate) const EITHER: Self = Self(1 | 2);

    const fn of(value: Option<bool>) -> Self {
        match value {
            Some(true) => Self::TRUE,
            Some(false) => Self::FALSE,
            None => Self::UNKNOWN,
        }
    }

    pub(crate) fn contains(self, value: Option<bool>) -> bool {
        self.0 & Self::of(value).0 != 0
    }

    fn values(self) -> impl Iterator<Item = Option<bool>> {
        Self::VALUES
            .into_iter()
            .filter(move |value| self.contains(*value))
    }

    /// Apply `op` to every pair of values from `self` and `other`.
    pub(crate) fn zip(
        self,
        other: Self,
        op: impl Fn(Option<bool>, Option<bool>) -> Option<bool>,
    ) -> Self {
        let mut result = Self(0);
        for left in self.values() {
            for right in other.values() {
                result.0 |= Self::of(op(left, right)).0;
            }
        }
        result
    }

    pub(crate) fn map(self, op: impl Fn(Option<bool>) -> Option<bool>) -> Self {
        self.zip(Self::TRUE, |value, _| op(value))
    }
}
