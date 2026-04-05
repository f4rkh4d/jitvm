//! cli stub. parsing only, for now. the actual run logic comes later.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = match args.first() {
        Some(p) => p,
        None => {
            eprintln!("usage: jitvm <file.jv>");
            return ExitCode::from(2);
        }
    };
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    match jitvm::parse(&src) {
        Ok(_) => {
            println!("parsed ok.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("parse: {e}");
            ExitCode::FAILURE
        }
    }
}
