//! hand-written lexer. produces a flat Vec<Token> with line/col.

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Int(i64),
    Ident(String),
    Fn,
    Let,
    If,
    Else,
    While,
    Return,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Semi,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Assign,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Bang,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
    pub col: u32,
}

pub fn tokenize(src: &str) -> Result<Vec<Token>> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut line: u32 = 1;
    let mut col: u32 = 1;

    while i < bytes.len() {
        let c = bytes[i] as char;

        if c == ' ' || c == '\t' || c == '\r' {
            i += 1;
            col += 1;
            continue;
        }
        if c == '\n' {
            i += 1;
            line += 1;
            col = 1;
            continue;
        }

        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        let start_line = line;
        let start_col = col;

        if c.is_ascii_digit() {
            let mut v: i64 = 0;
            while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                v = v
                    .checked_mul(10)
                    .and_then(|x| x.checked_add((bytes[i] - b'0') as i64))
                    .ok_or_else(|| {
                        Error::Parse(format!(
                            "int literal overflows i64 at {start_line}:{start_col}"
                        ))
                    })?;
                i += 1;
                col += 1;
            }
            out.push(Token {
                tok: Tok::Int(v),
                line: start_line,
                col: start_col,
            });
            continue;
        }

        if c.is_ascii_alphabetic() || c == '_' {
            let s_start = i;
            while i < bytes.len()
                && ((bytes[i] as char).is_ascii_alphanumeric() || bytes[i] == b'_')
            {
                i += 1;
                col += 1;
            }
            let s = &src[s_start..i];
            let tok = match s {
                "fn" => Tok::Fn,
                "let" => Tok::Let,
                "if" => Tok::If,
                "else" => Tok::Else,
                "while" => Tok::While,
                "return" => Tok::Return,
                _ => Tok::Ident(s.to_string()),
            };
            out.push(Token {
                tok,
                line: start_line,
                col: start_col,
            });
            continue;
        }

        let two = if i + 1 < bytes.len() {
            Some((c, bytes[i + 1] as char))
        } else {
            None
        };
        match two {
            Some(('=', '=')) => {
                push(&mut out, Tok::Eq, start_line, start_col);
                i += 2;
                col += 2;
                continue;
            }
            Some(('!', '=')) => {
                push(&mut out, Tok::Ne, start_line, start_col);
                i += 2;
                col += 2;
                continue;
            }
            Some(('<', '=')) => {
                push(&mut out, Tok::Le, start_line, start_col);
                i += 2;
                col += 2;
                continue;
            }
            Some(('>', '=')) => {
                push(&mut out, Tok::Ge, start_line, start_col);
                i += 2;
                col += 2;
                continue;
            }
            Some(('&', '&')) => {
                push(&mut out, Tok::AndAnd, start_line, start_col);
                i += 2;
                col += 2;
                continue;
            }
            Some(('|', '|')) => {
                push(&mut out, Tok::OrOr, start_line, start_col);
                i += 2;
                col += 2;
                continue;
            }
            _ => {}
        }

        let tok = match c {
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            '{' => Tok::LBrace,
            '}' => Tok::RBrace,
            ',' => Tok::Comma,
            ';' => Tok::Semi,
            '+' => Tok::Plus,
            '-' => Tok::Minus,
            '*' => Tok::Star,
            '/' => Tok::Slash,
            '%' => Tok::Percent,
            '=' => Tok::Assign,
            '<' => Tok::Lt,
            '>' => Tok::Gt,
            '!' => Tok::Bang,
            _ => {
                return Err(Error::Parse(format!(
                    "unexpected char {c:?} at {start_line}:{start_col}"
                )))
            }
        };
        push(&mut out, tok, start_line, start_col);
        i += 1;
        col += 1;
    }

    out.push(Token {
        tok: Tok::Eof,
        line,
        col,
    });
    Ok(out)
}

fn push(v: &mut Vec<Token>, tok: Tok, line: u32, col: u32) {
    v.push(Token { tok, line, col });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple() {
        let ts = tokenize("fn main() { return 42; }").unwrap();
        let kinds: Vec<_> = ts.iter().map(|t| t.tok.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                Tok::Fn,
                Tok::Ident("main".into()),
                Tok::LParen,
                Tok::RParen,
                Tok::LBrace,
                Tok::Return,
                Tok::Int(42),
                Tok::Semi,
                Tok::RBrace,
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn ops() {
        let ts = tokenize("a == b && c != d || !e").unwrap();
        assert_eq!(ts[1].tok, Tok::Eq);
        assert_eq!(ts[3].tok, Tok::AndAnd);
        assert_eq!(ts[5].tok, Tok::Ne);
        assert_eq!(ts[7].tok, Tok::OrOr);
        assert_eq!(ts[8].tok, Tok::Bang);
    }

    #[test]
    fn comment() {
        let ts = tokenize("// hi\n1").unwrap();
        assert_eq!(ts[0].tok, Tok::Int(1));
    }
}
