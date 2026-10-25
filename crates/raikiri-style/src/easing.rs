//! `<easing-function>` grammar (CSS Easing Functions 2 §2).

use cssparser::{Parser, ParserInput, Token};

type Result<T> = std::result::Result<T, ()>;

/// Whether `source` is exactly one `<easing-function>`.
#[doc(hidden)]
pub fn is_valid_easing_function(source: &str) -> bool {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parse_easing_function(&mut parser).is_ok() && parser.is_exhausted()
}

fn parse_easing_function(input: &mut Parser<'_, '_>) -> Result<()> {
    match input.next().map_err(drop)?.clone() {
        Token::Ident(name) => {
            const KEYWORDS: [&str; 7] = [
                "linear",
                "ease",
                "ease-in",
                "ease-out",
                "ease-in-out",
                "step-start",
                "step-end",
            ];
            KEYWORDS
                .iter()
                .any(|keyword| name.eq_ignore_ascii_case(keyword))
                .then_some(())
                .ok_or(())
        }
        Token::Function(name) if name.eq_ignore_ascii_case("cubic-bezier") => input
            .parse_nested_block(|input| {
                parse_cubic_bezier(input).map_err(|()| input.new_custom_error::<(), ()>(()))
            })
            .map_err(drop),
        Token::Function(name) if name.eq_ignore_ascii_case("steps") => input
            .parse_nested_block(|input| {
                parse_steps(input).map_err(|()| input.new_custom_error::<(), ()>(()))
            })
            .map_err(drop),
        Token::Function(name) if name.eq_ignore_ascii_case("linear") => input
            .parse_nested_block(|input| {
                parse_linear_stops(input).map_err(|()| input.new_custom_error::<(), ()>(()))
            })
            .map_err(drop),
        _ => Err(()),
    }
}

fn number(input: &mut Parser<'_, '_>) -> Result<f32> {
    input.expect_number().map_err(drop)
}

fn comma(input: &mut Parser<'_, '_>) -> Result<()> {
    input.expect_comma().map_err(drop)
}

/// `cubic-bezier(<number [0,1]>, <number>, <number [0,1]>, <number>)`.
fn parse_cubic_bezier(input: &mut Parser<'_, '_>) -> Result<()> {
    let x1 = number(input)?;
    comma(input)?;
    number(input)?;
    comma(input)?;
    let x2 = number(input)?;
    comma(input)?;
    number(input)?;
    ((0.0..=1.0).contains(&x1) && (0.0..=1.0).contains(&x2) && input.is_exhausted())
        .then_some(())
        .ok_or(())
}

/// `steps(<integer>, <step-position>?)`; `jump-none` needs two steps.
fn parse_steps(input: &mut Parser<'_, '_>) -> Result<()> {
    let steps = input.expect_integer().map_err(drop)?;
    let mut minimum = 1;
    if !input.is_exhausted() {
        comma(input)?;
        let position = input.expect_ident_cloned().map_err(drop)?;
        const POSITIONS: [&str; 6] = [
            "jump-start",
            "jump-end",
            "jump-none",
            "jump-both",
            "start",
            "end",
        ];
        if !POSITIONS
            .iter()
            .any(|keyword| position.eq_ignore_ascii_case(keyword))
        {
            return Err(());
        }
        if position.eq_ignore_ascii_case("jump-none") {
            minimum = 2;
        }
    }
    (steps >= minimum && input.is_exhausted())
        .then_some(())
        .ok_or(())
}

/// `linear([<number> && <percentage>{0,2}]#)` with at least two stops.
fn parse_linear_stops(input: &mut Parser<'_, '_>) -> Result<()> {
    let mut stops = 0;
    loop {
        let mut numbers = 0;
        let mut percentages = 0;
        let mut comma = false;
        while let Ok(token) = input.next().cloned() {
            match token {
                Token::Number { .. } if numbers == 0 => numbers += 1,
                Token::Percentage { .. } if percentages < 2 => percentages += 1,
                Token::Comma => {
                    comma = true;
                    break;
                }
                _ => return Err(()),
            }
        }
        if numbers != 1 {
            return Err(());
        }
        stops += 1;
        // A comma must be followed by another stop.
        if !comma {
            return (stops >= 2).then_some(()).ok_or(());
        }
    }
}

#[cfg(test)]
mod tests;
