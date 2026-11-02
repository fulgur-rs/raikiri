use super::{Length, LengthMath, Node};
use crate::property::{parse_length_value, product_operator, sum_operator};
use cssparser::{Parser, Token};
use std::sync::Arc;

const MAX_WORK: usize = 4096;

struct Budget {
    remaining: usize,
    comparison: bool,
}

enum Typed {
    Number(f32),
    Length(Node<Length>),
}

pub(super) fn parse(input: &mut Parser<'_, '_>) -> Option<Arc<LengthMath>> {
    input
        .try_parse(|input| {
            let start = input.state();
            if input.expect_function().is_err() {
                return Err(input.new_custom_error::<_, ()>(()));
            }
            input.reset(&start);
            let mut budget = Budget {
                remaining: MAX_WORK,
                comparison: false,
            };
            let value = budget
                .atom(input, 0)
                .ok_or_else(|| input.new_custom_error::<_, ()>(()))?;
            match value {
                Typed::Length(root) if budget.comparison => Ok(Arc::new(LengthMath { root })),
                _ => Err(input.new_custom_error(())),
            }
        })
        .ok()
}

impl Budget {
    fn step(&mut self) -> Option<()> {
        self.remaining = self.remaining.checked_sub(1)?;
        Some(())
    }

    fn sum(&mut self, input: &mut Parser<'_, '_>, depth: usize) -> Option<Typed> {
        let first = self.product(input, depth)?;
        let (mut number_value, mut terms) = match first {
            Typed::Number(value) => (Some(value), Vec::new()),
            Typed::Length(node) => (None, vec![node]),
        };
        while let Ok(operator) = input.try_parse(sum_operator) {
            self.step()?;
            match (number_value, self.product(input, depth)?) {
                (Some(left), Typed::Number(right)) => {
                    let value = if operator == '+' {
                        left + right
                    } else {
                        left - right
                    };
                    if !value.is_finite() {
                        return None;
                    }
                    number_value = Some(value);
                }
                (None, Typed::Length(right)) => {
                    terms.push(if operator == '+' {
                        right
                    } else {
                        scale(right, -1.0)?
                    });
                }
                _ => return None,
            }
        }
        match number_value {
            Some(value) => number(value),
            None if terms.len() == 1 => terms.pop().map(Typed::Length),
            None => Some(Typed::Length(Node::Sum(terms.into_boxed_slice()))),
        }
    }

    fn product(&mut self, input: &mut Parser<'_, '_>, depth: usize) -> Option<Typed> {
        let mut value = self.atom(input, depth)?;
        while let Ok(operator) = input.try_parse(product_operator) {
            self.step()?;
            let right = self.atom(input, depth)?;
            value = match (value, right, operator) {
                (Typed::Number(left), Typed::Number(right), '*') => number(left * right)?,
                (Typed::Number(left), Typed::Number(right), '/') if right != 0.0 => {
                    number(left / right)?
                }
                (Typed::Length(node), Typed::Number(factor), '*')
                | (Typed::Number(factor), Typed::Length(node), '*') => {
                    Typed::Length(scale(node, factor)?)
                }
                (Typed::Length(node), Typed::Number(divisor), '/') if divisor != 0.0 => {
                    Typed::Length(scale(node, 1.0 / divisor)?)
                }
                _ => return None,
            };
        }
        Some(value)
    }

    fn atom(&mut self, input: &mut Parser<'_, '_>, depth: usize) -> Option<Typed> {
        self.step()?;
        if depth > crate::MAX_CSS_NESTING_DEPTH {
            return None;
        }
        if let Ok(value) = input.try_parse(|input| input.expect_number()) {
            return number(value);
        }
        if let Ok(value) = input.try_parse(|input| {
            parse_length_value(input, true).ok_or_else(|| input.new_custom_error::<_, ()>(()))
        }) {
            return value
                .payload()
                .is_finite()
                .then_some(Typed::Length(Node::Leaf(value)));
        }
        let token = input.next().ok()?.clone();
        match token {
            Token::ParenthesisBlock => input
                .parse_nested_block(|nested| {
                    self.sum(nested, depth + 1)
                        .ok_or_else(|| nested.new_custom_error::<_, ()>(()))
                })
                .ok(),
            Token::Function(name) if name.eq_ignore_ascii_case("calc") => input
                .parse_nested_block(|nested| {
                    self.sum(nested, depth + 1)
                        .ok_or_else(|| nested.new_custom_error::<_, ()>(()))
                })
                .ok(),
            Token::Function(name)
                if ["min", "max", "clamp"]
                    .iter()
                    .any(|f| name.eq_ignore_ascii_case(f)) =>
            {
                self.comparison = true;
                let values = input
                    .parse_nested_block(|nested| {
                        nested.parse_comma_separated(|argument| {
                            self.sum(argument, depth + 1)
                                .ok_or_else(|| argument.new_custom_error::<_, ()>(()))
                        })
                    })
                    .ok()?;
                compare(&name, values)
            }
            _ => None,
        }
    }
}

fn number(value: f32) -> Option<Typed> {
    value.is_finite().then_some(Typed::Number(value))
}

fn scale(node: Node<Length>, factor: f32) -> Option<Node<Length>> {
    let (node, factor) = match node {
        Node::Scale(node, previous) => (*node, previous * factor),
        other => (other, factor),
    };
    factor
        .is_finite()
        .then(|| Node::Scale(Box::new(node), factor))
}

fn compare(name: &str, values: Vec<Typed>) -> Option<Typed> {
    if values.is_empty() || (name.eq_ignore_ascii_case("clamp") && values.len() != 3) {
        return None;
    }
    let numbers = values
        .iter()
        .map(|value| match value {
            Typed::Number(value) => Some(*value),
            Typed::Length(_) => None,
        })
        .collect::<Option<Vec<_>>>();
    if let Some(values) = numbers {
        let value = if name.eq_ignore_ascii_case("min") {
            values.into_iter().fold(f32::INFINITY, f32::min)
        } else if name.eq_ignore_ascii_case("max") {
            values.into_iter().fold(f32::NEG_INFINITY, f32::max)
        } else {
            values[0].max(values[1].min(values[2]))
        };
        return number(value);
    }
    let nodes = values
        .into_iter()
        .map(|value| match value {
            Typed::Length(node) => Some(node),
            Typed::Number(_) => None,
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Typed::Length(if name.eq_ignore_ascii_case("min") {
        Node::Min(nodes.into_boxed_slice())
    } else if name.eq_ignore_ascii_case("max") {
        Node::Max(nodes.into_boxed_slice())
    } else {
        Node::Clamp(Box::new(nodes.try_into().ok()?))
    }))
}
