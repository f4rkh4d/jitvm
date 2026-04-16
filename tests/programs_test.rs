//! for each tests/programs/*.jv, run it through the interpreter and (on
//! supported targets) the jit binary, and assert the stdout matches the
//! adjacent *.expected file.
//!
//! the jit case shells out to the built binary so each test runs in its own
//! process. running multiple jit compiles+executions in the same process
//! tripped a state-leak i haven't tracked down yet; subprocess isolation
//! sidesteps it and is closer to how the tool is actually used anyway.

use jitvm::{interp, Engine};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn collect_programs() -> Vec<(String, PathBuf, String, String)> {
    // (name, path, src, expected)
    let mut out = Vec::new();
    let dir = Path::new("tests/programs");
    for entry in fs::read_dir(dir).expect("read_dir tests/programs") {
        let e = entry.unwrap();
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) != Some("jv") {
            continue;
        }
        let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
        let src = fs::read_to_string(&p).unwrap();
        let exp_path = p.with_extension("expected");
        let expected = fs::read_to_string(&exp_path)
            .unwrap_or_else(|_| panic!("missing expected for {}", p.display()));
        out.push((stem, p, src, expected));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn normalize(s: &str) -> String {
    s.trim_end().to_string()
}

#[test]
fn interp_matches_expected_for_all_programs() {
    for (name, _path, src, expected) in collect_programs() {
        let eng = Engine::load_str(&src).unwrap_or_else(|e| panic!("parse {name}: {e}"));
        let out = interp::capture_prints(|| interp::run(&eng.program))
            .unwrap_or_else(|e| panic!("interp {name}: {e}"));
        let actual = out.join("\n");
        assert_eq!(
            normalize(&actual),
            normalize(&expected),
            "interp output mismatch for {name}"
        );
    }
}

#[cfg(all(target_arch = "x86_64", any(target_os = "linux", target_os = "macos")))]
#[test]
fn jit_matches_expected_for_all_programs() {
    // use the just-built binary. cargo sets CARGO_BIN_EXE_<name>.
    let bin = env!("CARGO_BIN_EXE_jitvm");
    for (name, path, _src, expected) in collect_programs() {
        let out = Command::new(bin)
            .arg("run")
            .arg(&path)
            .arg("--jit")
            .output()
            .unwrap_or_else(|e| panic!("spawn jit {name}: {e}"));
        assert!(
            out.status.success(),
            "jit run failed for {name}: status={:?} stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        let actual = String::from_utf8_lossy(&out.stdout).to_string();
        assert_eq!(
            normalize(&actual),
            normalize(&expected),
            "jit output mismatch for {name}"
        );
    }
}
