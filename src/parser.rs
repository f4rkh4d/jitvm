//! recursive-descent + pratt parser. top-level is a list of fn defs and/or
//! top-level statements; top-level stmts get bundled into a synthesized main().

use crate::ast::{Span, *};
use crate::lexer::{Tok, Token};
use crate::{Error, Result};

struct P<'a> {
    toks: &'a [Token],
    pos: usize,
}

impl<'a> P<'a> {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    fn line(&self) -> u32 {
        self.toks[self.pos].line
    }
    fn span(&self) -> Span {
        let t = &self.toks[self.pos];
        Span::new(t.line, t.col)
    }
    fn bump(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        self.pos += 1;
        t
    }
    fn eat(&mut self, want: &Tok) -> Result<()> {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(want) {
            self.bump();
            Ok(())
        } else {
            Err(Error::Parse(format!(
                "line {}: expected {:?}, got {:?}",
                self.line(),
                want,
                self.peek()
            )))
        }
    }
    fn eat_ident(&mut self) -> Result<String> {
        match self.bump() {
            Tok::Ident(s) => Ok(s),
            t => Err(Error::Parse(format!(
                "line {}: expected identifier, got {:?}",
                self.line(),
                t
            ))),
        }
    }
    fn maybe_semi(&mut self) {
        if matches!(self.peek(), Tok::Semi) {
            self.bump();
        }
    }
}

pub fn parse(toks: &[Token]) -> Result<Program> {
    let mut p = P { toks, pos: 0 };
    let mut fns = Vec::new();
    let mut top_stmts = Vec::new();
    while !matches!(p.peek(), Tok::Eof) {
        if matches!(p.peek(), Tok::Fn) {
            fns.push(parse_fn(&mut p)?);
        } else {
            top_stmts.push(parse_stmt(&mut p)?);
        }
    }
    // if user didn't define main, synthesize from top-level stmts.
    let has_main = fns.iter().any(|f| f.name == "main");
    if !top_stmts.is_empty() && !has_main {
        // synthesized main returns 0 implicitly.
        top_stmts.push(Stmt::Return(Expr::Int(0)));
        fns.push(Fn {
            name: "main".to_string(),
            params: Vec::new(),
            body: top_stmts,
        });
    } else if !top_stmts.is_empty() && has_main {
        return Err(Error::Parse(
            "top-level statements not allowed when main() is defined".into(),
        ));
    }
    if !fns.iter().any(|f| f.name == "main") {
        return Err(Error::Parse("no main function or top-level code".into()));
    }
    Ok(Program { fns })
}

fn parse_fn(p: &mut P) -> Result<Fn> {
    p.eat(&Tok::Fn)?;
    let name = p.eat_ident()?;
    p.eat(&Tok::LParen)?;
    let mut params = Vec::new();
    if !matches!(p.peek(), Tok::RParen) {
        loop {
            params.push(p.eat_ident()?);
            if matches!(p.peek(), Tok::Comma) {
                p.bump();
            } else {
                break;
            }
        }
    }
    p.eat(&Tok::RParen)?;
    let body = parse_block(p)?;
    Ok(Fn { name, params, body })
}

fn parse_block(p: &mut P) -> Result<Vec<Stmt>> {
    p.eat(&Tok::LBrace)?;
    let mut out = Vec::new();
    while !matches!(p.peek(), Tok::RBrace | Tok::Eof) {
        out.push(parse_stmt(p)?);
    }
    p.eat(&Tok::RBrace)?;
    Ok(out)
}

fn parse_stmt(p: &mut P) -> Result<Stmt> {
    match p.peek() {
        Tok::Let => {
            p.bump();
            let name = p.eat_ident()?;
            p.eat(&Tok::Assign)?;
            let e = parse_expr(p)?;
            p.maybe_semi();
            Ok(Stmt::Let(name, e))
        }
        Tok::If => {
            p.bump();
            let cond = parse_expr(p)?;
            let then_b = parse_block(p)?;
            let else_b = if matches!(p.peek(), Tok::Else) {
                p.bump();
                if matches!(p.peek(), Tok::If) {
                    vec![parse_stmt(p)?]
                } else {
                    parse_block(p)?
                }
            } else {
                Vec::new()
            };
            Ok(Stmt::If(cond, then_b, else_b))
        }
        Tok::While => {
            p.bump();
            let cond = parse_expr(p)?;
            let body = parse_block(p)?;
            Ok(Stmt::While(cond, body))
        }
        Tok::Return => {
            p.bump();
            let e = if matches!(p.peek(), Tok::RBrace | Tok::Semi | Tok::Eof) {
                Expr::Int(0)
            } else {
                parse_expr(p)?
            };
            p.maybe_semi();
            Ok(Stmt::Return(e))
        }
        Tok::Ident(name) if name == "print" => {
            p.bump();
            let e = parse_expr(p)?;
            p.maybe_semi();
            Ok(Stmt::Print(e))
        }
        Tok::Ident(_) => {
            let save = p.pos;
            if let Tok::Ident(name) = p.bump() {
                if matches!(p.peek(), Tok::Assign) {
                    p.bump();
                    let e = parse_expr(p)?;
                    p.maybe_semi();
                    return Ok(Stmt::Assign(name, e));
                }
            }
            p.pos = save;
            let e = parse_expr(p)?;
            p.maybe_semi();
            Ok(Stmt::ExprStmt(e))
        }
        _ => {
            let e = parse_expr(p)?;
            p.maybe_semi();
            Ok(Stmt::ExprStmt(e))
        }
    }
}

fn bin_info(t: &Tok) -> Option<(u8, BinOp)> {
    Some(match t {
        Tok::OrOr => (1, BinOp::Or),
        Tok::AndAnd => (2, BinOp::And),
        Tok::Eq => (3, BinOp::Eq),
        Tok::Ne => (3, BinOp::Ne),
        Tok::Lt => (4, BinOp::Lt),
        Tok::Le => (4, BinOp::Le),
        Tok::Gt => (4, BinOp::Gt),
        Tok::Ge => (4, BinOp::Ge),
        Tok::Plus => (5, BinOp::Add),
        Tok::Minus => (5, BinOp::Sub),
        Tok::Star => (6, BinOp::Mul),
        Tok::Slash => (6, BinOp::Div),
        Tok::Percent => (6, BinOp::Mod),
        _ => return None,
    })
}

fn parse_expr(p: &mut P) -> Result<Expr> {
    parse_prec(p, 0)
}

fn parse_prec(p: &mut P, min_bp: u8) -> Result<Expr> {
    let mut lhs = parse_unary(p)?;
    while let Some((lbp, op)) = bin_info(p.peek()) {
        if lbp < min_bp {
            break;
        }
        let op_span = p.span();
        p.bump();
        let rhs = parse_prec(p, lbp + 1)?;
        lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs), op_span);
    }
    Ok(lhs)
}

fn parse_unary(p: &mut P) -> Result<Expr> {
    match p.peek() {
        Tok::Minus => {
            p.bump();
            let e = parse_unary(p)?;
            Ok(Expr::Un(UnOp::Neg, Box::new(e)))
        }
        Tok::Bang => {
            p.bump();
            let e = parse_unary(p)?;
            Ok(Expr::Un(UnOp::Not, Box::new(e)))
        }
        _ => parse_atom(p),
    }
}

fn parse_atom(p: &mut P) -> Result<Expr> {
    let span = p.span();
    match p.bump() {
        Tok::Int(n) => Ok(Expr::Int(n)),
        Tok::StrLit(s) => Ok(Expr::Str(s, span)),
        Tok::LParen => {
            let e = parse_expr(p)?;
            p.eat(&Tok::RParen)?;
            Ok(e)
        }
        Tok::Ident(name) => {
            if matches!(p.peek(), Tok::LParen) {
                p.bump();
                let mut args = Vec::new();
                if !matches!(p.peek(), Tok::RParen) {
                    loop {
                        args.push(parse_expr(p)?);
                        if matches!(p.peek(), Tok::Comma) {
                            p.bump();
                        } else {
                            break;
                        }
                    }
                }
                p.eat(&Tok::RParen)?;
                Ok(Expr::Call(name, args, span))
            } else {
                Ok(Expr::Var(name))
            }
        }
        t => Err(Error::Parse(format!(
            "line {}: unexpected token {:?} in expression",
            p.line(),
            t
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;

    #[test]
    fn fib_parses() {
        let src = "fn fib(n) { if n < 2 { return n } return fib(n - 1) + fib(n - 2) } print fib(5)";
        let toks = tokenize(src).unwrap();
        let prog = parse(&toks).unwrap();
        assert!(prog.fns.iter().any(|f| f.name == "fib"));
        assert!(prog.fns.iter().any(|f| f.name == "main"));
    }
}
