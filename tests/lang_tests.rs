//! integration tests for the interpreter. if these pass, the language works.

use jitvm::{interp, Engine};

fn run_capture(src: &str) -> Vec<String> {
    let eng = Engine::load_str(src).expect("parse");
    interp::capture_prints(|| interp::run(&eng.program)).expect("run")
}

#[test]
fn arith() {
    assert_eq!(run_capture("print 1 + 2 * 3"), vec!["7"]);
    assert_eq!(run_capture("print (1 + 2) * 3"), vec!["9"]);
    assert_eq!(run_capture("print -5 + 10"), vec!["5"]);
    assert_eq!(run_capture("print 10 / 3"), vec!["3"]);
    assert_eq!(run_capture("print 10 % 3"), vec!["1"]);
}

#[test]
fn cmp_and_bool() {
    assert_eq!(run_capture("print 1 < 2"), vec!["1"]);
    assert_eq!(run_capture("print 2 < 1"), vec!["0"]);
    assert_eq!(run_capture("print 2 == 2"), vec!["1"]);
    assert_eq!(run_capture("print 2 != 3"), vec!["1"]);
    assert_eq!(run_capture("print !0"), vec!["1"]);
    assert_eq!(run_capture("print !5"), vec!["0"]);
}

#[test]
fn let_assign() {
    let src = "let x = 10 x = x + 1 print x";
    assert_eq!(run_capture(src), vec!["11"]);
}

#[test]
fn if_else() {
    let src = "let x = 5 if x < 10 { print 1 } else { print 2 }";
    assert_eq!(run_capture(src), vec!["1"]);
    let src = "let x = 50 if x < 10 { print 1 } else { print 2 }";
    assert_eq!(run_capture(src), vec!["2"]);
}

#[test]
fn else_if_chain() {
    let src = "let x = 2
        if x == 1 { print 10 } else if x == 2 { print 20 } else { print 30 }";
    assert_eq!(run_capture(src), vec!["20"]);
}

#[test]
fn while_loop() {
    let src = "let i = 0 let s = 0 while i < 10 { s = s + i i = i + 1 } print s";
    assert_eq!(run_capture(src), vec!["45"]);
}

#[test]
fn fn_call() {
    let src = "fn sq(n) { return n * n } print sq(5)";
    assert_eq!(run_capture(src), vec!["25"]);
}

#[test]
fn recursion_fact() {
    let src = "fn fact(n) { if n <= 1 { return 1 } return n * fact(n - 1) } print fact(10)";
    assert_eq!(run_capture(src), vec!["3628800"]);
}

#[test]
fn recursion_fib() {
    let src = "fn fib(n) { if n < 2 { return n } return fib(n - 1) + fib(n - 2) } print fib(15)";
    assert_eq!(run_capture(src), vec!["610"]);
}

#[test]
fn recursion_fib30() {
    let src = "fn fib(n) { if n < 2 { return n } return fib(n - 1) + fib(n - 2) } print fib(30)";
    assert_eq!(run_capture(src), vec!["832040"]);
}

#[test]
fn mutual_recursion() {
    let src = "
    fn is_even(n) { if n == 0 { return 1 } return is_odd(n - 1) }
    fn is_odd(n) { if n == 0 { return 0 } return is_even(n - 1) }
    print is_even(10)
    print is_odd(10)";
    assert_eq!(run_capture(src), vec!["1", "0"]);
}

#[test]
fn short_circuit_and() {
    // division by zero would kill us if the rhs were evaluated
    let src = "if 0 && (10 / 0) { print 1 } else { print 2 }";
    assert_eq!(run_capture(src), vec!["2"]);
}

#[test]
fn short_circuit_or() {
    let src = "if 1 || (10 / 0) { print 1 } else { print 2 }";
    assert_eq!(run_capture(src), vec!["1"]);
}

#[test]
fn nested_calls() {
    let src = "fn add(a, b) { return a + b } fn mul(a, b) { return a * b }
        print add(mul(2, 3), mul(4, 5))";
    assert_eq!(run_capture(src), vec!["26"]);
}

#[test]
fn negative_and_unary() {
    assert_eq!(run_capture("print -(-7)"), vec!["7"]);
    assert_eq!(run_capture("print --7"), vec!["7"]);
    assert_eq!(run_capture("let x = 5 print -x"), vec!["-5"]);
}

#[test]
fn comparison_chain_ops() {
    assert_eq!(run_capture("print 5 >= 5"), vec!["1"]);
    assert_eq!(run_capture("print 5 <= 4"), vec!["0"]);
    assert_eq!(run_capture("print 5 > 5"), vec!["0"]);
}

#[test]
fn while_with_early_return() {
    let src = "fn first_gt(n) {
        let i = 0
        while i < 100 { if i > n { return i } i = i + 1 }
        return -1
    }
    print first_gt(7)";
    assert_eq!(run_capture(src), vec!["8"]);
}

#[test]
fn many_locals() {
    let src = "let a = 1 let b = 2 let c = 3 let d = 4 let e = 5
        print a + b + c + d + e";
    assert_eq!(run_capture(src), vec!["15"]);
}

#[test]
fn comment_stripped() {
    let src = "// hi there\nprint 1 // trailing\n// another";
    assert_eq!(run_capture(src), vec!["1"]);
}

#[test]
fn return_no_value() {
    let src = "fn nop() { return } print nop() + 1";
    assert_eq!(run_capture(src), vec!["1"]);
}

#[test]
fn deep_recursion() {
    let src = "fn sum(n) { if n == 0 { return 0 } return n + sum(n - 1) } print sum(100)";
    assert_eq!(run_capture(src), vec!["5050"]);
}

#[test]
fn mod_negatives() {
    assert_eq!(run_capture("print 10 % 3"), vec!["1"]);
    assert_eq!(run_capture("print -10 % 3"), vec!["-1"]);
}
