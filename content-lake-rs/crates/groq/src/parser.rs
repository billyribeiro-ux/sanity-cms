use crate::ast::Expr;
use crate::lexer::{tokenize, LexError, SpannedToken, Token};

/// Parser error types.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("lex error: {0}")]
    Lex(#[from] LexError),
    #[error("unexpected token: {found}, expected: {expected}")]
    UnexpectedToken { found: String, expected: String },
    #[error("unexpected end of input")]
    UnexpectedEof,
}

/// Parse a GROQ query string into an AST.
pub fn parse(input: &str) -> Result<Expr, ParseError> {
    let tokens = tokenize(input)?;
    let mut parser = Parser::new(tokens);
    parser.parse_expr()
}

struct Parser {
    tokens: Vec<SpannedToken>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<SpannedToken>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens
            .get(self.pos)
            .map(|t| &t.token)
            .unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> &Token {
        let token = self
            .tokens
            .get(self.pos)
            .map(|t| &t.token)
            .unwrap_or(&Token::Eof);
        self.pos += 1;
        token
    }

    fn expect(&mut self, expected: &Token) -> Result<(), ParseError> {
        let found = self.advance().clone();
        if &found == expected {
            Ok(())
        } else {
            Err(ParseError::UnexpectedToken {
                found: format!("{found:?}"),
                expected: format!("{expected:?}"),
            })
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        match self.peek().clone() {
            Token::Star => {
                self.advance();
                if self.peek() == &Token::LBracket {
                    self.advance();
                    let filter = self.parse_filter_expr()?;
                    self.expect(&Token::RBracket)?;
                    let mut stages: Vec<Expr> =
                        vec![Expr::Everything, Expr::Filter(Box::new(filter))];
                    loop {
                        match self.peek() {
                            Token::LBrace => {
                                self.advance();
                                let projection = self.parse_projection()?;
                                self.expect(&Token::RBrace)?;
                                stages.push(Expr::Projection(projection));
                            }
                            Token::Pipe => {
                                self.advance();
                                let pipe = self.parse_pipe_expr()?;
                                stages.push(pipe);
                            }
                            Token::LBracket => {
                                self.advance();
                                let slice = self.parse_slice_body()?;
                                self.expect(&Token::RBracket)?;
                                stages.push(slice);
                            }
                            _ => break,
                        }
                    }
                    Ok(Expr::Pipeline(stages))
                } else {
                    Ok(Expr::Everything)
                }
            }
            _ => self.parse_filter_expr(),
        }
    }

    /// Parse the body of a `[...]` slice: either an integer index or a
    /// `start..end` / `start...end` range. Produces an [`Expr::Slice`].
    fn parse_slice_body(&mut self) -> Result<Expr, ParseError> {
        let start = match self.peek().clone() {
            Token::Integer(n) => {
                self.advance();
                n
            }
            other => {
                return Err(ParseError::UnexpectedToken {
                    found: format!("{other:?}"),
                    expected: "integer".to_string(),
                });
            }
        };
        match self.peek().clone() {
            Token::DotDot | Token::Ellipsis => {
                self.advance();
                let end = match self.peek().clone() {
                    Token::Integer(n) => {
                        self.advance();
                        n
                    }
                    other => {
                        return Err(ParseError::UnexpectedToken {
                            found: format!("{other:?}"),
                            expected: "integer".to_string(),
                        });
                    }
                };
                Ok(Expr::Slice(Box::new(Expr::This), start, end))
            }
            _ => {
                // Single-element index `[n]` — represent as [n..n].
                Ok(Expr::Slice(Box::new(Expr::This), start, start))
            }
        }
    }

    fn parse_filter_expr(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_comparison()?;

        match self.peek().clone() {
            Token::And => {
                self.advance();
                let right = self.parse_filter_expr()?;
                Ok(Expr::And(Box::new(left), Box::new(right)))
            }
            Token::Or => {
                self.advance();
                let right = self.parse_filter_expr()?;
                Ok(Expr::Or(Box::new(left), Box::new(right)))
            }
            _ => Ok(left),
        }
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_primary()?;

        match self.peek().clone() {
            Token::Eq => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::Eq(Box::new(left), Box::new(right)))
            }
            Token::Neq => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::Neq(Box::new(left), Box::new(right)))
            }
            Token::Lt => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::Lt(Box::new(left), Box::new(right)))
            }
            Token::Gt => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::Gt(Box::new(left), Box::new(right)))
            }
            Token::Lte => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::Lte(Box::new(left), Box::new(right)))
            }
            Token::Gte => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::Gte(Box::new(left), Box::new(right)))
            }
            Token::In => {
                self.advance();
                let right = self.parse_primary()?;
                Ok(Expr::In(Box::new(left), Box::new(right)))
            }
            Token::Match => {
                self.advance();
                let right = self.parse_primary()?;
                // Represent as FuncCall so sql_gen's existing match handler picks it up.
                Ok(Expr::FuncCall("match".to_string(), vec![left, right]))
            }
            _ => Ok(left),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().clone() {
            Token::Ident(name) => {
                self.advance();
                let mut expr = Expr::Ident(name);
                // Handle dot access chains: a.b.c
                while self.peek() == &Token::Dot {
                    self.advance();
                    match self.peek().clone() {
                        Token::Ident(field) => {
                            self.advance();
                            expr = Expr::DotAccess(Box::new(expr), field);
                        }
                        _ => break,
                    }
                }
                // Handle dereference: a->b
                if self.peek() == &Token::Arrow {
                    self.advance();
                    if let Token::Ident(field) = self.peek().clone() {
                        self.advance();
                        expr = Expr::Deref(Box::new(expr), field);
                    }
                }
                // Handle function calls: fn(args)
                if self.peek() == &Token::LParen {
                    if let Expr::Ident(fn_name) = &expr {
                        let fn_name = fn_name.clone();
                        self.advance();
                        let mut args = Vec::new();
                        if self.peek() != &Token::RParen {
                            // `parse_expr` (not `parse_filter_expr`) so function
                            // arguments can contain `*[...]` pipelines, e.g.
                            // `count(*[_type == "page"])`.
                            args.push(self.parse_expr()?);
                            while self.peek() == &Token::Comma {
                                self.advance();
                                args.push(self.parse_expr()?);
                            }
                        }
                        self.expect(&Token::RParen)?;
                        expr = Expr::FuncCall(fn_name, args);
                    }
                }
                Ok(expr)
            }
            Token::String(s) => {
                self.advance();
                Ok(Expr::StringLiteral(s))
            }
            Token::Integer(n) => {
                self.advance();
                Ok(Expr::IntLiteral(n))
            }
            Token::Float(n) => {
                self.advance();
                Ok(Expr::FloatLiteral(n))
            }
            Token::Bool(b) => {
                self.advance();
                Ok(Expr::BoolLiteral(b))
            }
            Token::Null => {
                self.advance();
                Ok(Expr::Null)
            }
            Token::At => {
                self.advance();
                Ok(Expr::This)
            }
            Token::Caret => {
                self.advance();
                Ok(Expr::Parent)
            }
            Token::Param(name) => {
                self.advance();
                Ok(Expr::Param(name))
            }
            Token::Not => {
                self.advance();
                let expr = self.parse_primary()?;
                Ok(Expr::Not(Box::new(expr)))
            }
            Token::LParen => {
                self.advance();
                let expr = self.parse_filter_expr()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            Token::LBracket => {
                self.advance();
                let mut items = Vec::new();
                if self.peek() != &Token::RBracket {
                    items.push(self.parse_filter_expr()?);
                    while self.peek() == &Token::Comma {
                        self.advance();
                        items.push(self.parse_filter_expr()?);
                    }
                }
                self.expect(&Token::RBracket)?;
                Ok(Expr::Array(items))
            }
            Token::Star => {
                self.advance();
                Ok(Expr::Everything)
            }
            Token::Eof => Err(ParseError::UnexpectedEof),
            other => Err(ParseError::UnexpectedToken {
                found: format!("{other:?}"),
                expected: "expression".to_string(),
            }),
        }
    }

    fn parse_projection(&mut self) -> Result<Vec<(String, Expr)>, ParseError> {
        let mut fields = Vec::new();

        if self.peek() == &Token::Ellipsis {
            self.advance();
            fields.push(("...".to_string(), Expr::Everything));
            if self.peek() == &Token::Comma {
                self.advance();
            }
        }

        while self.peek() != &Token::RBrace && self.peek() != &Token::Eof {
            if self.peek() == &Token::Ellipsis {
                self.advance();
                fields.push(("...".to_string(), Expr::Everything));
            } else if let Token::String(alias) = self.peek().clone() {
                self.advance();
                self.expect(&Token::Colon)?;
                let expr = self.parse_filter_expr()?;
                fields.push((alias, expr));
            } else if let Token::Ident(name) = self.peek().clone() {
                self.advance();
                if self.peek() == &Token::Colon {
                    self.advance();
                    let expr = self.parse_filter_expr()?;
                    fields.push((name, expr));
                } else if self.peek() == &Token::Arrow {
                    // `author->` or `author->field` — dereferenced projection.
                    self.advance();
                    let base = Expr::Ident(name.clone());
                    let (alias, expr) = if let Token::Ident(field) = self.peek().clone() {
                        self.advance();
                        (name, Expr::Deref(Box::new(base), field))
                    } else {
                        // Bare `author->` means "follow the ref but return the
                        // whole target doc". Represent as Deref with empty field.
                        (name, Expr::Deref(Box::new(base), String::new()))
                    };
                    fields.push((alias, expr));
                } else {
                    fields.push((name.clone(), Expr::Ident(name)));
                }
            } else {
                break;
            }

            if self.peek() == &Token::Comma {
                self.advance();
            }
        }

        Ok(fields)
    }

    fn parse_pipe_expr(&mut self) -> Result<Expr, ParseError> {
        if let Token::Ident(name) = self.peek().clone() {
            match name.as_str() {
                "order" => {
                    self.advance();
                    self.expect(&Token::LParen)?;
                    let field = self.parse_primary()?;
                    let ascending = if self.peek() == &Token::Desc {
                        self.advance();
                        false
                    } else {
                        if self.peek() == &Token::Asc {
                            self.advance();
                        }
                        true
                    };
                    self.expect(&Token::RParen)?;
                    Ok(Expr::Order(Box::new(field), ascending))
                }
                _ => self.parse_filter_expr(),
            }
        } else {
            self.parse_filter_expr()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_everything() {
        let expr = parse("*").unwrap();
        assert!(matches!(expr, Expr::Everything));
    }

    #[test]
    fn parse_simple_filter() {
        let expr = parse("*[_type == \"post\"]").unwrap();
        match expr {
            Expr::Pipeline(stages) => {
                assert_eq!(stages.len(), 2);
                assert!(matches!(stages[0], Expr::Everything));
                match &stages[1] {
                    Expr::Filter(inner) => match inner.as_ref() {
                        Expr::Eq(left, right) => {
                            assert!(matches!(left.as_ref(), Expr::Ident(n) if n == "_type"));
                            assert!(
                                matches!(right.as_ref(), Expr::StringLiteral(s) if s == "post")
                            );
                        }
                        _ => panic!("expected Eq"),
                    },
                    _ => panic!("expected Filter"),
                }
            }
            _ => panic!("expected Pipeline"),
        }
    }

    #[test]
    fn parse_boolean_logic() {
        let expr = parse("*[_type == \"post\" && published == true]").unwrap();
        match expr {
            Expr::Pipeline(stages) => {
                assert_eq!(stages.len(), 2);
                match &stages[1] {
                    Expr::Filter(inner) => {
                        assert!(matches!(inner.as_ref(), Expr::And(_, _)));
                    }
                    _ => panic!("expected Filter"),
                }
            }
            _ => panic!("expected Pipeline"),
        }
    }

    #[test]
    fn parse_dot_access() {
        let expr = parse("slug.current").unwrap();
        match expr {
            Expr::DotAccess(base, field) => {
                assert!(matches!(*base, Expr::Ident(n) if n == "slug"));
                assert_eq!(field, "current");
            }
            _ => panic!("expected DotAccess, got {expr:?}"),
        }
    }

    #[test]
    fn parse_function_call() {
        let expr = parse("count(*)").unwrap();
        match expr {
            Expr::FuncCall(name, args) => {
                assert_eq!(name, "count");
                assert_eq!(args.len(), 1);
            }
            _ => panic!("expected FuncCall"),
        }
    }
}
