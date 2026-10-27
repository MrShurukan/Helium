use super::*;

impl Parser<'_> {
    pub(super) fn import_declaration(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        let start = tokens.previous()?.source_location;
        let extension = matches!(self.peek(tokens)?.token_type, TT::Keyword(KT::Extension));
        if extension {
            tokens.advance();
        }
        let mut path = vec![];
        loop {
            let part =
                check_and_consume!(self, tokens, TT::Identifier(_), "Expected import path").clone();
            let TT::Identifier(name) = part.token_type else {
                unreachable!()
            };
            path.push(name);
            if !matches!(self.peek(tokens)?.token_type, TT::Punctuation(PT::Colon)) {
                break;
            }
            tokens.advance();
            check_and_consume!(
                self,
                tokens,
                TT::Punctuation(PT::Colon),
                "Expected '::' in import path"
            );
        }
        let end = check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::Semicolon),
            "Expected ';' after import"
        )
        .source_location;
        Some(Statement {
            symbol: None,
            source_location: crate::frontend::utils::SourceLocation::new_from_start_end(start, end),
            statement_type: StatementType::Import { path, extension },
        })
    }
    pub(super) fn public_declaration(&mut self, tokens: &mut TokenIterator) -> Option<Statement> {
        let token = self.advance(tokens)?.clone();
        let mut declaration = match token.token_type {
            TT::Keyword(KT::Fn) => self.function_declaration(tokens)?,
            TT::Keyword(KT::NoInterrupt) => self.modified_function(tokens).map(|mut s| {
                if let StatementType::FunctionDeclaration { no_interrupt, .. } =
                    &mut s.statement_type
                {
                    *no_interrupt = Some(token.source_location);
                }
                s
            })?,
            TT::Keyword(KT::Extension) => self.extension_declaration(tokens)?,
            _ => {
                self.errors.push(self.get_error_marked_line(
                    ParserErrorType::MissingToken(
                        "Expected function or extension after pub".into(),
                    ),
                    token,
                ));
                return None;
            }
        };
        match &mut declaration.statement_type {
            StatementType::FunctionDeclaration { public, .. }
            | StatementType::Extension { public, .. } => *public = true,
            _ => unreachable!(),
        }
        Some(declaration)
    }
    pub(super) fn extension_declaration(
        &mut self,
        tokens: &mut TokenIterator,
    ) -> Option<Statement> {
        let start = tokens.previous()?.source_location;
        let name =
            check_and_consume!(self, tokens, TT::Identifier(_), "Expected extension name").clone();
        check_and_consume!(
            self,
            tokens,
            TT::Keyword(KT::For),
            "Expected 'for' after extension name"
        );
        let kind =
            check_and_consume!(self, tokens, TT::Identifier(_), "Expected receiver kind").clone();
        check_and_consume!(
            self,
            tokens,
            TT::Punctuation(PT::LeftBrace),
            "Expected extension body"
        );
        let mut methods = vec![];
        while !matches!(
            self.peek(tokens)?.token_type,
            TT::Punctuation(PT::RightBrace)
        ) {
            let token = self.advance(tokens)?.clone();
            let method = match token.token_type {
                TT::Keyword(KT::Pub) => self.public_declaration(tokens)?,
                TT::Keyword(KT::Fn) => self.function_declaration(tokens)?,
                TT::Keyword(KT::NoInterrupt) => {
                    let mut s = self.modified_function(tokens)?;
                    if let StatementType::FunctionDeclaration { no_interrupt, .. } =
                        &mut s.statement_type
                    {
                        *no_interrupt = Some(token.source_location);
                    }
                    s
                }
                _ => {
                    self.errors.push(self.get_error_marked_line(
                        ParserErrorType::MissingToken("Extensions contain only functions".into()),
                        token,
                    ));
                    return None;
                }
            };
            if !matches!(
                method.statement_type,
                StatementType::FunctionDeclaration { .. }
            ) {
                self.errors
                    .push(self.missing_token_error("Extensions contain only functions", tokens));
                return None;
            }
            methods.push(method);
        }
        let end = self.advance(tokens)?.source_location;
        let TT::Identifier(identifier) = name.token_type else {
            unreachable!()
        };
        let TT::Identifier(receiver) = kind.token_type else {
            unreachable!()
        };
        Some(Statement {
            symbol: None,
            source_location: crate::frontend::utils::SourceLocation::new_from_start_end(start, end),
            statement_type: StatementType::Extension {
                public: false,
                identifier,
                receiver,
                methods,
            },
        })
    }
}
