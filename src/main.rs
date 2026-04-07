//! cli. interp only for now. jit comes later.

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
    let eng = match jitvm::Engine::load_file(path) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match eng.run_interp() {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
