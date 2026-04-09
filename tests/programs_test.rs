//! for each tests/programs/*.jv, run it through the interpreter and compare
//! stdout to the adjacent *.expected file. jit variant lands later.

use jitvm::{interp, Engine};
use std::fs;
use std::path::Path;

fn collect() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let dir = Path::new("tests/programs");
    for entry in fs::read_dir(dir).expect("read_dir tests/programs") {
        let p = entry.unwrap().path();
        if p.extension().and_then(|s| s.to_str()) != Some("jv") {
            continue;
        }
        let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
        let src = fs::read_to_string(&p).unwrap();
        let expected = fs::read_to_string(p.with_extension("expected"))
            .unwrap_or_else(|_| panic!("missing expected for {}", p.display()));
        out.push((stem, src, expected));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn interp_matches_expected_for_all_programs() {
    for (name, src, expected) in collect() {
        let eng = Engine::load_str(&src).unwrap_or_else(|e| panic!("parse {name}: {e}"));
        let out = interp::capture_prints(|| interp::run(&eng.program))
            .unwrap_or_else(|e| panic!("interp {name}: {e}"));
        assert_eq!(
            out.join("\n").trim_end(),
            expected.trim_end(),
            "interp output mismatch for {name}"
        );
    }
}
