//! jitvm command-line interface.
//!
//! subcommands:
//!   run <file.jv> [--interp]  run via jit (default) or interpreter.
//!   bench <file.jv>           run both, report timings, ratio.
//!   disasm <file.jv>          dump bytecode.
//!   repl                      interactive.

use std::env;
use std::process::ExitCode;
use std::time::Instant;

fn usage() {
    eprintln!(
        "usage:\n  jitvm run <file.jv> [--interp]\n  jitvm bench <file.jv>\n  jitvm disasm <file.jv>\n  jitvm repl"
    );
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        usage();
        return ExitCode::from(2);
    }
    match args[1].as_str() {
        "run" => cmd_run(&args[2..]),
        "bench" => cmd_bench(&args[2..]),
        "disasm" => cmd_disasm(&args[2..]),
        "repl" => cmd_repl(),
        "--help" | "-h" => {
            usage();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("unknown subcommand: {other}");
            usage();
            ExitCode::from(2)
        }
    }
}

fn load(path: &str) -> Result<jitvm::Engine, ExitCode> {
    jitvm::Engine::load_file(path).map_err(|e| {
        eprintln!("error: {e}");
        ExitCode::from(1)
    })
}

fn cmd_run(args: &[String]) -> ExitCode {
    if args.is_empty() {
        usage();
        return ExitCode::from(2);
    }
    let force_interp = args.iter().any(|a| a == "--interp");
    let eng = match load(&args[0]) {
        Ok(e) => e,
        Err(c) => return c,
    };
    let r;
    #[cfg(all(target_arch = "x86_64", any(target_os = "linux", target_os = "macos")))]
    {
        r = if force_interp {
            eng.run_interp()
        } else {
            eng.run_jit()
        };
    }
    #[cfg(not(all(target_arch = "x86_64", any(target_os = "linux", target_os = "macos"))))]
    {
        let _ = force_interp;
        r = eng.run_interp();
    }
    match r {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

fn cmd_disasm(args: &[String]) -> ExitCode {
    if args.is_empty() {
        usage();
        return ExitCode::from(2);
    }
    let eng = match load(&args[0]) {
        Ok(e) => e,
        Err(c) => return c,
    };
    for (i, f) in eng.program.fns.iter().enumerate() {
        let mark = if i as u32 == eng.program.main_id {
            "*"
        } else {
            " "
        };
        println!(
            "{mark} fn {} (argc={}, locals={}, ops={})",
            f.name,
            f.argc,
            f.locals,
            f.code.len()
        );
        for (k, op) in f.code.iter().enumerate() {
            println!("    {k:04}  {op:?}");
        }
    }
    ExitCode::SUCCESS
}

fn cmd_bench(args: &[String]) -> ExitCode {
    if args.is_empty() {
        usage();
        return ExitCode::from(2);
    }
    let path = &args[0];
    let eng = match load(path) {
        Ok(e) => e,
        Err(c) => return c,
    };
    println!("{}", path);

    let t = Instant::now();
    let vm_captured = match jitvm::interp::capture_prints(|| jitvm::interp::run(&eng.program)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let vm_dt = t.elapsed();
    let vm_last = vm_captured
        .last()
        .cloned()
        .unwrap_or_else(|| "0".to_string());

    #[cfg(all(target_arch = "x86_64", any(target_os = "linux", target_os = "macos")))]
    {
        let module = match jitvm::jit::compile(&eng.program) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(1);
            }
        };
        let t = Instant::now();
        let jit_captured = match jitvm::jit::capture_prints(|| module.run_main()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(1);
            }
        };
        let jit_dt = t.elapsed();
        let jit_last = jit_captured
            .last()
            .cloned()
            .unwrap_or_else(|| "0".to_string());
        let ratio = vm_dt.as_nanos() as f64 / jit_dt.as_nanos().max(1) as f64;
        println!("  vm   {} in {:?}", vm_last, vm_dt);
        println!("  jit  {} in {:?}  ({:.1}x)", jit_last, jit_dt, ratio);
        ExitCode::SUCCESS
    }
    #[cfg(not(all(target_arch = "x86_64", any(target_os = "linux", target_os = "macos"))))]
    {
        println!("  vm   {} in {:?}", vm_last, vm_dt);
        println!("  jit  unavailable on this platform");
        ExitCode::SUCCESS
    }
}

fn cmd_repl() -> ExitCode {
    use std::io::{self, BufRead, Write};
    let stdin = io::stdin();
    let mut out = io::stdout();
    let mut accum = String::new();
    println!("jitvm repl. :q to quit, :reset, :disasm");
    loop {
        print!("> ");
        out.flush().ok();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if l == ":q" || l == ":quit" {
            break;
        }
        if l == ":reset" {
            accum.clear();
            continue;
        }
        if l == ":disasm" {
            let src = format!("{accum}\nfn __n() {{ return 0 }}");
            match jitvm::Engine::load_str(&src) {
                Ok(e) => {
                    for f in &e.program.fns {
                        if f.name == "__n" {
                            continue;
                        }
                        println!("fn {} ({} ops)", f.name, f.code.len());
                        for (k, op) in f.code.iter().enumerate() {
                            println!("  {k:04} {op:?}");
                        }
                    }
                }
                Err(e) => println!("err: {e}"),
            }
            continue;
        }
        let src = if l.starts_with("fn ") {
            accum.push_str(l);
            accum.push('\n');
            continue;
        } else {
            format!("{accum}\nfn main() {{ print {l} return 0 }}")
        };
        match jitvm::Engine::load_str(&src) {
            Ok(e) => {
                let r = e.run_interp();
                if let Err(err) = r {
                    println!("err: {err}");
                }
            }
            Err(e) => println!("err: {e}"),
        }
    }
    ExitCode::SUCCESS
}
