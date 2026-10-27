//! Stop fragment tree building on node overflow and suspend at the next tag.

use std::borrow::Cow;

use html5ever::buffer_queue::BufferQueue;
use html5ever::interface::QualName;
use html5ever::tendril::{StrTendril, TendrilSink, fmt::UTF8};
use html5ever::tokenizer::{Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts};
use html5ever::tree_builder::{TreeBuilder, TreeSink};
use markup5ever::TokenizerResult;

use crate::sink::RaikiriTreeSink;
use crate::types::UncascadedDocument;

struct BudgetedBuilder(TreeBuilder<usize, RaikiriTreeSink>);

impl TokenSink for BudgetedBuilder {
    type Handle = usize;

    fn process_token(&self, token: Token, line: u64) -> TokenSinkResult<usize> {
        // html5ever permits suspension only from tag tokens; character,
        // comment, error, and EOF callbacks must return Continue.
        let may_suspend = matches!(&token, Token::TagToken(_));
        if self.0.sink.dom_node_limit_error().is_some() {
            return if may_suspend {
                TokenSinkResult::Script(0)
            } else {
                TokenSinkResult::Continue
            };
        }
        let result = self.0.process_token(token, line);
        if may_suspend && self.0.sink.dom_node_limit_error().is_some() {
            // Reuse html5ever's suspension boundary; the driver never resumes
            // a tokenizer suspended by the node budget.
            TokenSinkResult::Script(0)
        } else {
            result
        }
    }

    fn end(&self) {
        self.0.end();
    }

    fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
        self.0
            .adjusted_current_node_present_but_not_in_html_namespace()
    }
}

pub(super) struct BudgetedFragmentParser {
    tokenizer: Tokenizer<BudgetedBuilder>,
    input: BufferQueue,
}

impl BudgetedFragmentParser {
    pub(super) fn new(sink: RaikiriTreeSink, name: QualName, scripting: bool) -> Self {
        let context = markup5ever::interface::create_element(&sink, name, Vec::new());
        let builder = TreeBuilder::new_for_fragment(sink, context, None, Default::default());
        let options = TokenizerOpts {
            initial_state: Some(builder.tokenizer_state_for_context_elem(scripting)),
            ..Default::default()
        };
        Self {
            tokenizer: Tokenizer::new(BudgetedBuilder(builder), options),
            input: BufferQueue::default(),
        }
    }

    fn pump(&self) {
        while self.tokenizer.sink.0.sink.dom_node_limit_error().is_none() {
            if matches!(self.tokenizer.feed(&self.input), TokenizerResult::Done) {
                break;
            }
        }
    }
}

impl TendrilSink<UTF8> for BudgetedFragmentParser {
    type Output = UncascadedDocument;

    fn process(&mut self, input: StrTendril) {
        self.input.push_back(input);
        self.pump();
    }

    fn error(&mut self, error: Cow<'static, str>) {
        self.tokenizer.sink.0.sink.parse_error(error);
    }

    fn finish(self) -> Self::Output {
        self.pump();
        self.tokenizer.end();
        self.tokenizer.sink.0.sink.finish()
    }
}

#[cfg(test)]
mod tests;
