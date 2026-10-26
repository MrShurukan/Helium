use super::*;
use crate::frontend::{statement::TimerDeclaration, utils::SourceLocation};

impl Parser<'_> {
    fn timer_duration(&mut self, tokens: &mut TokenIterator) -> Option<(u64, SourceLocation)> {
        let number = check_and_consume!(
            self,
            tokens,
            TT::Literal(LT::Integer(_)),
            "Expected integer duration, for example 1s or 500ms"
        )
        .clone();
        let unit = check_and_consume!(
            self,
            tokens,
            TT::Identifier(_),
            "Expected duration unit: ns, us, ms or s"
        )
        .clone();
        let (TT::Literal(LT::Integer(value)), TT::Identifier(unit_name)) =
            (&number.token_type, &unit.token_type)
        else {
            unreachable!()
        };
        let Some(nanos) = crate::timer::duration(*value, unit_name) else {
            self.errors.push(self.get_error_marked_line(
                ParserErrorType::MissingToken(
                    "Invalid duration unit or overflowing duration; use integer ns/us/ms/s".into(),
                ),
                unit,
            ));
            return None;
        };
        Some((
            nanos,
            SourceLocation::new_from_start_end(number.source_location, unit.source_location),
        ))
    }

    pub(super) fn timer_declaration(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        let start = tokens.previous()?.source_location;
        let name =
            check_and_consume!(self, tokens, TT::Identifier(_), "Expected timer name").clone();
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::Equal),
            "Expected '=' after timer name"
        );
        let hardware = check_and_consume!(
            self,
            tokens,
            TT::Identifier(_),
            "Expected hardware timer, for example Timer1"
        )
        .clone();
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::LeftBrace),
            "Expected '{' after hardware timer"
        );
        let mut every = None;
        let mut tolerance = None;
        let mut handler = None;
        loop {
            if matches!(
                self.peek(tokens)?.token_type,
                TT::Punctuation(PT::RightBrace)
            ) {
                break;
            }
            let property = self.advance(tokens)?.clone();
            match &property.token_type {
                TT::Keyword(KT::Interrupt) if handler.is_none() => {
                    check_and_consume!(
                        self,
                        tokens,
                        TT::Punctuation(PT::LeftBrace),
                        "Expected interrupt body"
                    );
                    let block = self.block_statement(tokens)?;
                    let StatementType::Block(body) = block.statement_type else {
                        unreachable!()
                    };
                    handler = Some((body, property.source_location));
                }
                TT::Identifier(field)
                    if (field == "every" && every.is_none())
                        || (field == "tolerance" && tolerance.is_none()) =>
                {
                    check_and_consume!(
                        self,
                        tokens,
                        TT::Punctuation(PT::Colon),
                        "Expected ':' after timer option"
                    );
                    let value = self.timer_duration(tokens)?;
                    if field == "every" {
                        every = Some(value);
                    } else {
                        tolerance = Some(value);
                    }
                    check_and_consume!(
                        self,
                        tokens,
                        TT::Punctuation(PT::Comma),
                        "Expected ',' after timer option"
                    );
                }
                _ => {
                    self.errors.push(self.get_error_marked_line(
                        ParserErrorType::MissingToken(
                            "Expected a unique every, tolerance or interrupt entry".into(),
                        ),
                        property,
                    ));
                    return None;
                }
            }
        }
        let end = self.advance(tokens)?.source_location;
        let (Some((every_ns, period_location)), Some((body, handler_location))) = (every, handler)
        else {
            self.errors.push(self.missing_token_error_previous(
                "Timer requires every and exactly one interrupt block",
                tokens,
            ));
            return None;
        };
        let TT::Identifier(identifier) = name.token_type else {
            unreachable!()
        };
        let TT::Identifier(hardware_name) = hardware.token_type else {
            unreachable!()
        };
        Some(Statement {
            symbol: None,
            source_location: SourceLocation::new_from_start_end(start, end),
            statement_type: StatementType::TimerDeclaration(TimerDeclaration {
                identifier,
                name_location: name.source_location,
                hardware: hardware_name,
                hardware_location: hardware.source_location,
                period_location,
                every_ns,
                tolerance_ns: tolerance.map_or(0, |v| v.0),
                body,
                handler_location,
            }),
        })
    }
}
