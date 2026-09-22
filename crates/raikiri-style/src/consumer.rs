//! Registration metadata for consumer-owned CSS properties.
//!
//! The style crate keeps the registration here because it is the CSS parser's
//! leaf.  The umbrella crate re-exports the small registration vocabulary, but
//! resolved events never expose any style implementation type.

use smol_str::SmolStr;

/// Grammar used to validate a registered consumer-owned property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConsumerPropertyGrammar {
    /// A signed CSS integer.
    Integer,
    /// A signed CSS integer or the `none` keyword.
    IntegerOrNone,
    /// A CSS content list that is resolved to neutral text by the producer.
    Text,
}

/// Registration for one consumer-owned CSS property.
///
/// Names are ASCII-lowercased and may be written with or without the leading
/// `--`.  A registration for `bookmark-level` therefore accepts both the
/// consumer-facing declaration `bookmark-level: 1` and the custom-property
/// spelling `--bookmark-level: 1`.  The latter is useful when the same value
/// is referenced through CSS `var()` syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConsumerPropertyRegistration {
    name: SmolStr,
    grammar: ConsumerPropertyGrammar,
    inherits: bool,
}

impl ConsumerPropertyRegistration {
    /// Register an integer-valued property.
    pub fn integer(name: impl AsRef<str>) -> Self {
        Self::new(name, ConsumerPropertyGrammar::Integer)
    }

    /// Register an integer-valued property that also accepts `none`.
    pub fn integer_or_none(name: impl AsRef<str>) -> Self {
        Self::new(name, ConsumerPropertyGrammar::IntegerOrNone)
    }

    /// Register a resolved-text property.
    pub fn text(name: impl AsRef<str>) -> Self {
        Self::new(name, ConsumerPropertyGrammar::Text)
    }

    /// Construct a registration with an explicit neutral grammar.
    pub fn new(name: impl AsRef<str>, grammar: ConsumerPropertyGrammar) -> Self {
        let name = name
            .as_ref()
            .trim()
            .trim_start_matches("--")
            .to_ascii_lowercase();
        Self {
            name: SmolStr::new(name),
            grammar,
            // Consumer properties are local by default.  This matches the
            // non-inherited semantics of bookmark-* while allowing a caller
            // to opt into inherited values explicitly.
            inherits: false,
        }
    }

    /// Property name reported in resolved events, without a CSS `--` prefix.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Value grammar used by this registration.
    pub fn grammar(&self) -> ConsumerPropertyGrammar {
        self.grammar
    }

    /// Make the property inherit its resolved value when no local declaration
    /// exists on a node.
    pub fn inherited(mut self) -> Self {
        self.inherits = true;
        self
    }

    /// Keep the default local-only behavior explicit at call sites.
    pub fn non_inherited(mut self) -> Self {
        self.inherits = false;
        self
    }

    /// Whether an effective inherited value should be observed.
    pub fn inherits(&self) -> bool {
        self.inherits
    }

    /// Storage name used by the existing custom-property cascade path.
    pub(crate) fn storage_name(&self) -> SmolStr {
        let mut name = String::with_capacity(self.name.len() + 2);
        name.push_str("--");
        name.push_str(self.name.as_str());
        SmolStr::new(name)
    }

    /// Find the registration corresponding to a CSS declaration name.
    pub(crate) fn matches_css_name(&self, css_name: &str) -> bool {
        css_name
            .strip_prefix("--")
            .unwrap_or(css_name)
            .eq_ignore_ascii_case(self.name.as_str())
    }
}
