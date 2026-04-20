//! error-path tests. parse errors are checked by substring since we want
//! to be able to reword them without breaking the suite; runtime errors
//! are pinned by variant via `Error::is_runtime()` and by the presence of
//! a source span in the formatted message where relevant.

use jitvm::{interp, Engine, Error};

fn parse_err(src: &str) -> String {
    match Engine::load_str(src) {
        Ok(_) => panic!("expected parse error, got ok"),
        Err(Error::Parse(s)) => s,
        Err(e) => panic!("expected Parse, got {e:?}"),
    }
}

fn runtime_err(src: &str) -> Error {
    let eng = Engine::load_str(src).expect("parse ok");
    let r = interp::capture_prints(|| interp::run(&eng.program));
    match r {
        Ok(_) => panic!("expected runtime error, got ok"),
        Err(e) if e.is_runtime() => e,
        Err(e) => panic!("expected Runtime, got {e:?}"),
    }
}

#[test]
fn parse_error_reports_line_number() {
    // two blank lines, then a ')' that has no matching '('. the offending
    // token sits on line 3; we pin that the error message names line 3.
    let src = "\n\n)";
    let msg = parse_err(src);
    assert!(
        msg.contains("line 3") || msg.contains("3:"),
        "expected line 3 mention in parse error, got: {msg}"
    );
}

#[test]
fn parse_error_on_unexpected_char() {
    let msg = parse_err("let x = 1 @ 2");
    assert!(
        msg.contains("unexpected"),
        "expected 'unexpected' in: {msg}"
    );
}

#[test]
fn parse_error_on_int_overflow() {
    // 10^20 is way past i64::MAX
    let msg = parse_err("print 100000000000000000000");
    assert!(
        msg.contains("overflow") || msg.contains("overflows"),
        "expected overflow mention: {msg}"
    );
}

#[test]
fn parse_error_on_too_many_args() {
    // 7 params; we cap call-site arity at 6.
    let src = "fn f(a,b,c,d,e,f,g) { return 0 } print f(1,2,3,4,5,6,7)";
    let msg = parse_err(src);
    assert!(
        msg.contains(">6") || msg.contains("6"),
        "expected arity limit mention: {msg}"
    );
}

#[test]
fn div_by_zero_is_a_runtime_error_with_span() {
    // the `/` lives on line 2, col 11 (1-indexed, the '/' character).
    let src = "let a = 7\nprint a / 0\n";
    let e = runtime_err(src);
    let msg = format!("{e}");
    assert!(msg.contains("division by zero"), "got: {msg}");
    assert!(
        msg.contains("line 2"),
        "expected span in runtime error, got: {msg}"
    );
}

#[test]
fn mod_by_zero_is_a_runtime_error_with_span() {
    let src = "print 10 % 0";
    let e = runtime_err(src);
    let msg = format!("{e}");
    assert!(msg.contains("mod by zero"), "got: {msg}");
    assert!(msg.contains("line 1"), "got: {msg}");
}

#[test]
fn undefined_variable_is_a_parse_error() {
    // variables are resolved at lowering time, not at runtime, so this is
    // a parse-shaped error.
    let msg = parse_err("print xyz");
    assert!(
        msg.contains("undeclared") || msg.contains("unknown"),
        "got: {msg}"
    );
}

#[test]
fn unknown_function_is_a_parse_error() {
    let msg = parse_err("print nope(1)");
    assert!(
        msg.contains("unknown") || msg.contains("undefined"),
        "got: {msg}"
    );
}

#[test]
fn top_level_stmts_with_main_is_a_parse_error() {
    let src = "fn main() { print 1 }\nprint 2";
    let msg = parse_err(src);
    assert!(
        msg.contains("top-level") || msg.contains("main"),
        "got: {msg}"
    );
}
