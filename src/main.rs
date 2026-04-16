//! cli. `jitvm run <file> [--jit]`. bench/disasm come next.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (path, use_jit) = match args.as_slice() {
        [s, p] | [p, s] if s == "run" || s == "--jit" => {
            let use_jit = s == "--jit" || args.iter().any(|a| a == "--jit");
            (p.clone(), use_jit)
        }
        [p] => (p.clone(), false),
        _ => {
            eprintln!("usage: jitvm run <file.jv> [--jit]");
            return ExitCode::from(2);
        }
    };
    let eng = match jitvm::Engine::load_file(&path) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let r = if use_jit { eng.run_jit() } else { eng.run_interp() };
    match r {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => { eprintln!("{e}"); ExitCode::FAILURE }
    }
}
