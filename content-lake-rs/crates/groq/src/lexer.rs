use std::fmt;

use serde::{Deserialize, Serialize};

/// Token types produced by the GROQ lexer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Token {
    /// A string literal.
    String(String),
    /// An integer literal.
    Integer(i64),
    /// A floating-point literal.
    Float(f64),
    /// A boolean literal.
    Bool(bool),
    /// The null literal.
    Null,

    /// An identifier.
    Ident(String),

    /// The equality operator.
    Eq, // ==
    /// The inequality operator.
    Neq, // !=
    /// The less-than operator.
    Lt, // <
    /// The greater-than operator.
    Gt, // >
    /// The less-than-or-equal operator.
    Lte, // <=
    /// The greater-than-or-equal operator.
    Gte, // >=
    /// The logical and operator.
    And, // &&
    /// The logical or operator.
    Or, // ||
    /// The logical not operator.
    Not, // !
    /// The match keyword.
    Match, // match
    /// The in keyword.
    In, // in
    /// The asc keyword.
    Asc, // asc
    /// The desc keyword.
    Desc, // desc

    /// The asterisk operator.
    Star, // *
    /// The dot operator.
    Dot, // .
    /// The comma operator.
    Comma, // ,
    /// The colon operator.
    Colon, // :
    /// The pipe operator.
    Pipe, // |
    /// The arrow operator.
    Arrow, // ->
    /// The at symbol.
    At, // @
    /// The caret operator.
    Caret, // ^
    /// The ellipsis operator.
    Ellipsis, // ...

    /// The left parenthesis.
    LParen, // (
    /// The right parenthesis.
    RParen, // )
    /// The left bracket.
    LBracket, // [
    /// The right bracket.
    RBracket, // ]
    /// The left brace.
    LBrace, // {
    /// The right brace.
    RBrace, // }

    /// The end of the input.
    Eof,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::String(s) => write!(f, "\"{s}\""),
            Token::Integer(n) => write!(f, "{n}"),
            Token::Float(n) => write!(f, "{n}"),
            Token::Bool(b) => write!(f, "{b}"),
            Token::Null => write!(f, "null"),
            Token::Ident(s) => write!(f, "{s}"),
            Token::Star => write!(f, "*"),
            Token::Dot => write!(f, "."),
            Token::Eof => write!(f, "EOF"),
            other => write!(f, "{other:?}"),
        }
    }
}

/// Position in source code for error reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// A token with its source position.
#[derive(Debug, Clone, PartialEq)]
pub struct SpannedToken {
    pub token: Token,
    pub span: Span,
}

/// Lexer error.
#[derive(Debug, thiserror::Error)]
pub enum LexError {
    #[error("unexpected character '{0}' at position {1}")]
    UnexpectedChar(char, usize),
    #[error("unterminated string starting at position {0}")]
    UnterminatedString(usize),
}

/// Tokenize a GROQ query string into a sequence of tokens.
pub fn tokenize(input: &str) -> Result<Vec<SpannedToken>, LexError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut pos = 0;

    while pos < chars.len() {
        let ch = chars[pos];

        // Skip whitespace
        if ch.is_whitespace() {
            pos += 1;
            continue;
        }

        // Skip single-line comments
        if ch == '/' && pos + 1 < chars.len() && chars[pos + 1] == '/' {
            while pos < chars.len() && chars[pos] != '\n' {
                pos += 1;
            }
            continue;
        }

        let start = pos;

        let token = match ch {
            '*' => {
                pos += 1;
                Token::Star
            }
            '.' => {
                if pos + 2 < chars.len() && chars[pos + 1] == '.' && chars[pos + 2] == '.' {
                    pos += 3;
                    Token::Ellipsis
                } else {
                    pos += 1;
                    Token::Dot
                }
            }
            ',' => {
                pos += 1;
                Token::Comma
            }
            ':' => {
                pos += 1;
                Token::Colon
            }
            '@' => {
                pos += 1;
                Token::At
            }
            '^' => {
                pos += 1;
                Token::Caret
            }
            '(' => {
                pos += 1;
                Token::LParen
            }
            ')' => {
                pos += 1;
                Token::RParen
            }
            '[' => {
                pos += 1;
                Token::LBracket
            }
            ']' => {
                pos += 1;
                Token::RBracket
            }
            '{' => {
                pos += 1;
                Token::LBrace
            }
            '}' => {
                pos += 1;
                Token::RBrace
            }
            '=' => {
                if pos + 1 < chars.len() && chars[pos + 1] == '=' {
                    pos += 2;
                    Token::Eq
                } else {
                    return Err(LexError::UnexpectedChar(ch, pos));
                }
            }
            '!' => {
                if pos + 1 < chars.len() && chars[pos + 1] == '=' {
                    pos += 2;
                    Token::Neq
                } else {
                    pos += 1;
                    Token::Not
                }
            }
            '<' => {
                if pos + 1 < chars.len() && chars[pos + 1] == '=' {
                    pos += 2;
                    Token::Lte
                } else {
                    pos += 1;
                    Token::Lt
                }
            }
            '>' => {
                if pos + 1 < chars.len() && chars[pos + 1] == '=' {
                    pos += 2;
                    Token::Gte
                } else {
                    pos += 1;
                    Token::Gt
                }
            }
            '&' => {
                if pos + 1 < chars.len() && chars[pos + 1] == '&' {
                    pos += 2;
                    Token::And
                } else {
                    return Err(LexError::UnexpectedChar(ch, pos));
                }
            }
            '|' => {
                if pos + 1 < chars.len() && chars[pos + 1] == '|' {
                    pos += 2;
                    Token::Or
                } else {
                    pos += 1;
                    Token::Pipe
                }
            }
            '-' => {
                if pos + 1 < chars.len() && chars[pos + 1] == '>' {
                    pos += 2;
                    Token::Arrow
                } else if pos + 1 < chars.len() && chars[pos + 1].is_ascii_digit() {
                    // Negative number
                    pos += 1;
                    let num_start = pos;
                    let mut is_float = false;
                    while pos < chars.len() && (chars[pos].is_ascii_digit() || chars[pos] == '.') {
                        if chars[pos] == '.' {
                            is_float = true;
                        }
                        pos += 1;
                    }
                    let num_str = &input[num_start..pos];
                    if is_float {
                        Token::Float(-num_str.parse::<f64>().unwrap())
                    } else {
                        Token::Integer(-num_str.parse::<i64>().unwrap())
                    }
                } else {
                    return Err(LexError::UnexpectedChar(ch, pos));
                }
            }
            '"' | '\'' => {
                let quote = ch;
                pos += 1;
                let str_start = pos;
                while pos < chars.len() && chars[pos] != quote {
                    if chars[pos] == '\\' {
                        pos += 1; // skip escaped char
                    }
                    pos += 1;
                }
                if pos >= chars.len() {
                    return Err(LexError::UnterminatedString(start));
                }
                let s = input[str_start..pos].to_string();
                pos += 1; // skip closing quote
                Token::String(s)
            }
            c if c.is_ascii_digit() => {
                let mut is_float = false;
                while pos < chars.len() && (chars[pos].is_ascii_digit() || chars[pos] == '.') {
                    if chars[pos] == '.' {
                        // Check for .. (range) vs . (decimal)
                        if pos + 1 < chars.len() && chars[pos + 1] == '.' {
                            break;
                        }
                        is_float = true;
                    }
                    pos += 1;
                }
                let num_str = &input[start..pos];
                if is_float {
                    Token::Float(num_str.parse().unwrap())
                } else {
                    Token::Integer(num_str.parse().unwrap())
                }
            }
            c if c.is_alphabetic() || c == '_' || c == '$' => {
                while pos < chars.len() && (chars[pos].is_alphanumeric() || chars[pos] == '_') {
                    pos += 1;
                }
                let word = &input[start..pos];
                match word {
                    "true" => Token::Bool(true),
                    "false" => Token::Bool(false),
                    "null" => Token::Null,
                    "match" => Token::Match,
                    "in" => Token::In,
                    "asc" => Token::Asc,
                    "desc" => Token::Desc,
                    _ => Token::Ident(word.to_string()),
                }
            }
            _ => return Err(LexError::UnexpectedChar(ch, pos)),
        };

        tokens.push(SpannedToken {
            token,
            span: Span { start, end: pos },
        });
    }

    tokens.push(SpannedToken {
        token: Token::Eof,
        span: Span {
            start: pos,
            end: pos,
        },
    });

    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(input: &str) -> Vec<Token> {
        tokenize(input)
            .unwrap()
            .into_iter()
            .map(|t| t.token)
            .collect()
    }

    #[test]
    fn tokenize_simple_filter() {
        let tokens = tok("*[_type == \"post\"]");
        assert_eq!(tokens[0], Token::Star);
        assert_eq!(tokens[1], Token::LBracket);
        assert_eq!(tokens[2], Token::Ident("_type".into()));
        assert_eq!(tokens[3], Token::Eq);
        assert_eq!(tokens[4], Token::String("post".into()));
        assert_eq!(tokens[5], Token::RBracket);
        assert_eq!(tokens[6], Token::Eof);
    }

    #[test]
    fn tokenize_projection() {
        let tokens = tok("{title, \"slug\": slug.current}");
        assert_eq!(tokens[0], Token::LBrace);
        assert_eq!(tokens[1], Token::Ident("title".into()));
        assert_eq!(tokens[2], Token::Comma);
        assert_eq!(tokens[3], Token::String("slug".into()));
        assert_eq!(tokens[4], Token::Colon);
        assert_eq!(tokens[5], Token::Ident("slug".into()));
        assert_eq!(tokens[6], Token::Dot);
        assert_eq!(tokens[7], Token::Ident("current".into()));
        assert_eq!(tokens[8], Token::RBrace);
    }

    #[test]
    fn tokenize_numbers() {
        let tokens = tok("42 3.125 -7");
        assert_eq!(tokens[0], Token::Integer(42));
        assert_eq!(tokens[1], Token::Float(3.125));
        assert_eq!(tokens[2], Token::Integer(-7));
    }

    #[test]
    fn tokenize_comparison_operators() {
        let tokens = tok("< > <= >= == != !");
        assert_eq!(tokens[0], Token::Lt);
        assert_eq!(tokens[1], Token::Gt);
        assert_eq!(tokens[2], Token::Lte);
        assert_eq!(tokens[3], Token::Gte);
        assert_eq!(tokens[4], Token::Eq);
        assert_eq!(tokens[5], Token::Neq);
        assert_eq!(tokens[6], Token::Not);
    }

    #[test]
    fn tokenize_keywords() {
        let tokens = tok("true false null match in asc desc");
        assert_eq!(tokens[0], Token::Bool(true));
        assert_eq!(tokens[1], Token::Bool(false));
        assert_eq!(tokens[2], Token::Null);
        assert_eq!(tokens[3], Token::Match);
        assert_eq!(tokens[4], Token::In);
        assert_eq!(tokens[5], Token::Asc);
        assert_eq!(tokens[6], Token::Desc);
    }

    #[test]
    fn tokenize_dereference() {
        let tokens = tok("author->name");
        assert_eq!(tokens[0], Token::Ident("author".into()));
        assert_eq!(tokens[1], Token::Arrow);
        assert_eq!(tokens[2], Token::Ident("name".into()));
    }

    #[test]
    fn tokenize_ellipsis() {
        let tokens = tok("{...}");
        assert_eq!(tokens[0], Token::LBrace);
        assert_eq!(tokens[1], Token::Ellipsis);
        assert_eq!(tokens[2], Token::RBrace);
    }

    #[test]
    fn unterminated_string_error() {
        let result = tokenize("\"hello");
        assert!(result.is_err());
    }
}
