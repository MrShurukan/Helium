use super::*;
use crate::frontend::{statement::DeviceDeclaration, utils::SourceLocation};

impl Parser<'_> {
    pub(super) fn device_declaration(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        let start = tokens.previous()?.source_location;
        let name =
            check_and_consume!(self, tokens, TT::Identifier(_), "Expected device name").clone();
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::Equal),
            "Expected '=' after device name"
        );
        let hardware = check_and_consume!(
            self,
            tokens,
            TT::Identifier(_),
            "Expected hardware resource"
        )
        .clone();
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::LeftBrace),
            "Expected device configuration"
        );
        let mut baud = None;
        let mut mode = false;
        while !matches!(
            self.peek(tokens)?.token_type,
            TT::Punctuation(PT::RightBrace)
        ) {
            let field =
                check_and_consume!(self, tokens, TT::Identifier(_), "Expected baud or mode")
                    .clone();
            check_and_consume!(
                self,
                tokens,
                TT::Punctuation(PT::Colon),
                "Expected ':' after device option"
            );
            match &field.token_type {
                TT::Identifier(value) if value == "baud" && baud.is_none() => {
                    let number = check_and_consume!(
                        self,
                        tokens,
                        TT::Literal(LT::Integer(_)),
                        "Expected integer baud rate"
                    )
                    .clone();
                    let TT::Literal(LT::Integer(value)) = number.token_type else {
                        unreachable!()
                    };
                    baud = Some((value, number.source_location));
                }
                TT::Identifier(value) if value == "mode" && !mode => {
                    let value =
                        check_and_consume!(self, tokens, TT::Identifier(_), "Expected TX mode")
                            .clone();
                    if !matches!(&value.token_type, TT::Identifier(name) if name == "TX") {
                        self.errors.push(self.get_error_marked_line(
                            ParserErrorType::MissingToken("Only TX mode is supported".into()),
                            value,
                        ));
                        return None;
                    }
                    mode = true;
                }
                _ => {
                    self.errors.push(self.get_error_marked_line(
                        ParserErrorType::MissingToken(
                            "Expected a unique baud or mode option".into(),
                        ),
                        field,
                    ));
                    return None;
                }
            }
            check_and_consume!(
                self,
                tokens,
                TT::Punctuation(PT::Comma),
                "Expected ',' after device option"
            );
        }
        let end = self.advance(tokens)?.source_location;
        let Some((baud, baud_location)) = baud.filter(|_| mode) else {
            self.errors.push(
                self.missing_token_error_previous("Device requires baud and mode: TX", tokens),
            );
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
            statement_type: StatementType::DeviceDeclaration(DeviceDeclaration {
                identifier,
                hardware: hardware_name,
                baud,
                name_location: name.source_location,
                hardware_location: hardware.source_location,
                baud_location,
            }),
        })
    }
}
