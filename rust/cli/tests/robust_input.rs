//! Bad input is an error, never a hang, a panic or a silent pass (meshweights-3; the fix is
//! meshweights-4).
//!
//! - A number the tool reads from the GLB that is not finite (NaN in a vertex position or a
//!   bind matrix) stops `check` and `fix` with exit 2 and a message that says so. On
//!   2026-11-19 one NaN in a bind matrix made `weights check` run for ever on the 0.1 s test
//!   mannequin (Mac and Linux), and a NaN position ran for ever on a Mac and panicked on Linux
//!   (`metrics.rs:207`, an unwrap). A NaN skin weight is a broken weight, which is what `check`
//!   judges: it fails the rig (exit 1, the vertex unweighted), and must keep doing so.
//! - A result file that can't be written is an error (non-zero exit, the file named on
//!   stderr), not exit 0 with the file missing: `show`'s findings.json and `blind-score`'s
//!   verdict.json were written with `let _ =`.
//!
//! (glTF JSON can't hold a NaN, so a node's translation can't be one; the binary buffers can.)
//! Each run gets a deadline: a command still running after it is killed and the test fails
//! saying it hung.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(90);

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_weights"))
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wf_robust_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Out {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    hung: bool,
}

/// Runs `weights` with a deadline; past it the process is killed and `hung` is set.
fn run(args: &[&str]) -> Out {
    let mut child = Command::new(bin()).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let (mut so, mut se) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let ro = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = so.read_to_string(&mut s);
        s
    });
    let re = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = se.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let mut hung = false;
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break Some(st);
        }
        if start.elapsed() > DEADLINE {
            let _ = child.kill();
            let _ = child.wait();
            hung = true;
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    Out { code: status.and_then(|s| s.code()), stdout: ro.join().unwrap(), stderr: re.join().unwrap(), hung }
}

fn fixture(d: &Path, fault: &str) -> PathBuf {
    let o = run(&["fixture", fault, "--out", d.to_str().unwrap()]);
    assert_eq!(o.code, Some(0), "fixture {fault}: {}", o.stderr);
    d.join(format!("mannequin_{fault}.glb"))
}

/// The GLB's JSON chunk and the byte offset of its BIN chunk's data.
fn chunks(b: &[u8]) -> (serde_json::Value, usize) {
    assert_eq!(&b[0..4], b"glTF");
    let jl = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
    let j: serde_json::Value = serde_json::from_slice(&b[20..20 + jl]).unwrap();
    (j, 20 + jl + 8)
}

/// Byte offset in the file of an accessor's first element.
fn accessor_at(j: &serde_json::Value, bin: usize, acc: usize) -> usize {
    let a = &j["accessors"][acc];
    let bv = &j["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
    bin + bv["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize
}

/// A copy of `src` with NaN written over `n` floats starting `skip` floats into the named
/// data: "position" (the first mesh's POSITION), "weight" (its WEIGHTS_0, which must be
/// floats) or "bind" (skin 0's inverse bind matrices).
fn with_nan(src: &Path, dst: &Path, what: &str, skip: usize, n: usize) {
    let mut b = std::fs::read(src).unwrap();
    let (j, bin) = chunks(&b);
    let prim = &j["meshes"][0]["primitives"][0]["attributes"];
    let acc = match what {
        "position" => prim["POSITION"].as_u64().unwrap(),
        "weight" => prim["WEIGHTS_0"].as_u64().unwrap(),
        "bind" => j["skins"][0]["inverseBindMatrices"].as_u64().unwrap(),
        _ => unreachable!(),
    } as usize;
    assert_eq!(j["accessors"][acc]["componentType"], 5126, "{what} must be float for this test");
    let at = accessor_at(&j, bin, acc);
    for k in 0..n {
        let o = at + (skip + k) * 4;
        b[o..o + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    }
    std::fs::write(dst, b).unwrap();
}

fn says_not_finite(o: &Out) -> bool {
    let e = format!("{}{}", o.stderr, o.stdout);
    e.contains("NaN") || e.contains("finite")
}

/// The usual outcome of a bad file: exit 2, a message, no panic, no hang.
fn assert_input_error(o: &Out, what: &str) {
    assert!(!o.hung, "{what}: still running after {DEADLINE:?} (hung on bad input)");
    assert!(!o.stderr.contains("panicked"), "{what}: panicked\n{}", o.stderr);
    assert_eq!(o.code, Some(2), "{what}: exit {:?}, want 2 (bad input)\nstdout:\n{}\nstderr:\n{}", o.code, o.stdout, o.stderr);
    assert!(says_not_finite(o), "{what}: the message doesn't say a number was not finite (NaN):\n{}", o.stderr);
}

#[test]
fn nan_position_is_an_input_error() {
    let d = tmp("nan_position");
    let clean = fixture(&d, "clean");
    let bad = d.join("nan_position.glb");
    with_nan(&clean, &bad, "position", 30, 1);
    assert_input_error(&run(&["check", bad.to_str().unwrap()]), "check, NaN position");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn nan_bind_matrix_is_an_input_error() {
    let d = tmp("nan_bind");
    let clean = fixture(&d, "clean");
    // One component of joint 3's bind matrix (its translation's x).
    let one = d.join("nan_bind_one.glb");
    with_nan(&clean, &one, "bind", 3 * 16 + 12, 1);
    assert_input_error(&run(&["check", one.to_str().unwrap()]), "check, one NaN in a bind matrix");
    // A whole matrix.
    let all = d.join("nan_bind_all.glb");
    with_nan(&clean, &all, "bind", 3 * 16, 16);
    assert_input_error(&run(&["check", all.to_str().unwrap(), "--json"]), "check --json, a NaN bind matrix");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn nan_weight_fails_the_rig() {
    let d = tmp("nan_weight");
    let clean = fixture(&d, "clean");
    let bad = d.join("nan_weight.glb");
    with_nan(&clean, &bad, "weight", 40, 1);
    let o = run(&["check", bad.to_str().unwrap()]);
    assert!(!o.hung, "check, NaN weight: hung");
    assert!(!o.stderr.contains("panicked"), "check, NaN weight: panicked\n{}", o.stderr);
    assert_eq!(o.code, Some(1), "check, NaN weight: exit {:?}, want 1 (a failing rig)\n{}", o.code, o.stderr);
    assert!(o.stdout.contains("D_UNWEIGHTED") || says_not_finite(&o), "check, NaN weight: not named\n{}", o.stdout);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn fix_refuses_nan_and_writes_nothing() {
    let d = tmp("fix_nan");
    let bleed = fixture(&d, "bleed");
    for (what, skip, n) in [("position", 30, 1), ("bind", 3 * 16 + 12, 1)] {
        let bad = d.join(format!("nan_{what}.glb"));
        with_nan(&bleed, &bad, what, skip, n);
        let out = d.join(format!("fixed_{what}.glb"));
        let o = run(&["fix", bad.to_str().unwrap(), "--out", out.to_str().unwrap()]);
        assert_input_error(&o, &format!("fix, NaN {what}"));
        assert!(!out.exists(), "fix, NaN {what}: wrote {} anyway", out.display());
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// The control: the same files without the NaN still pass and fail as before.
#[test]
fn clean_inputs_still_work() {
    let d = tmp("control");
    let clean = fixture(&d, "clean");
    let bleed = fixture(&d, "bleed");
    let o = run(&["check", clean.to_str().unwrap()]);
    assert!(!o.hung && o.code == Some(0), "check clean: {:?}\n{}", o.code, o.stderr);
    let o = run(&["check", bleed.to_str().unwrap()]);
    assert!(!o.hung && o.code == Some(1), "check bleed: {:?}\n{}", o.code, o.stderr);
    // NaN written over zero floats: byte-identical, same verdict.
    let same = d.join("same.glb");
    with_nan(&clean, &same, "position", 0, 0);
    assert_eq!(std::fs::read(&same).unwrap(), std::fs::read(&clean).unwrap());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn show_fails_when_findings_json_cant_be_written() {
    let d = tmp("show_write");
    let bleed = fixture(&d, "bleed");
    let out = d.join("show");
    std::fs::create_dir_all(out.join("findings.json")).unwrap(); // a folder where the file goes
    let o = run(&["show", bleed.to_str().unwrap(), "--out", out.to_str().unwrap(), "--top", "2"]);
    assert!(!o.hung, "show hung");
    // It got as far as the sheets (so the write really was reached) ...
    let pngs = std::fs::read_dir(&out).unwrap().filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "png")).count();
    assert!(pngs > 0, "show wrote no sheets (exit {:?}):\n{}", o.code, o.stderr);
    // ... and then reports the file it couldn't write.
    assert_ne!(o.code, Some(0), "show exited 0 without writing findings.json\n{}", o.stdout);
    assert!(o.stderr.contains("findings.json"), "show's error doesn't name findings.json:\n{}", o.stderr);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn blind_score_fails_when_verdict_json_cant_be_written() {
    let d = tmp("verdict_write");
    let pack = d.join("pairs");
    std::fs::create_dir_all(pack.join("verdict.json")).unwrap(); // a folder where the file goes
    let pair = |n: u64, category: &str, left: &str, right: &str, expected: &str| {
        serde_json::json!({"pair": n, "category": category, "focus": "left upper arm", "frame": "walk 0.50",
            "left_rig": left, "right_rig": right, "expected": expected, "tears_cm2": {"A": 1.0, "B": 2.0},
            "tears_winner": "A", "score_winner": "A"})
    };
    let key = serde_json::json!({"rig_a": "a.glb", "rig_b": "b.glb", "section": "clips", "seed": 7, "planted": "swap",
        "tears_pct": {"A": 1.0, "B": 2.0}, "scores": {"A": 80.0, "B": 70.0},
        "pairs": [pair(1, "claim", "A", "B", "A"), pair(2, "control", "A", "A", "cant_tell"), pair(3, "planted", "B", "P", "B")]});
    std::fs::write(pack.join("answer_key.json"), key.to_string()).unwrap();
    let answers = d.join("answers.json");
    std::fs::write(&answers, r#"{"pair_01": "left", "pair_02": "cant_tell", "pair_03": "left"}"#).unwrap();
    let o = run(&["blind-score", pack.to_str().unwrap(), "--answers", answers.to_str().unwrap()]);
    assert!(!o.hung, "blind-score hung");
    assert!(o.stdout.contains("pair 01"), "blind-score didn't score the pairs (exit {:?}):\n{}{}", o.code, o.stdout, o.stderr);
    assert_ne!(o.code, Some(0), "blind-score exited 0 without writing verdict.json\n{}", o.stdout);
    assert!(o.stderr.contains("verdict.json"), "blind-score's error doesn't name verdict.json:\n{}", o.stderr);
    let _ = std::fs::remove_dir_all(&d);
}

/// Writes that already report their failure keep doing so.
#[test]
fn fix_and_dump_report_unwritable_outputs() {
    let d = tmp("fix_write");
    let bleed = fixture(&d, "bleed");
    let b = bleed.to_str().unwrap();
    let missing = d.join("no_such_folder").join("fixed.glb");
    let o = run(&["fix", b, "--out", missing.to_str().unwrap()]);
    assert!(!o.hung && o.code == Some(2), "fix into a missing folder: {:?}\n{}", o.code, o.stderr);
    let out = d.join("fixed.glb");
    std::fs::create_dir_all(d.join("fixed.report.json")).unwrap();
    let o = run(&["fix", b, "--out", out.to_str().unwrap()]);
    assert!(!o.hung && o.code != Some(0), "fix with its report blocked exited 0\n{}", o.stdout);
    assert!(o.stderr.contains("report.json"), "fix's error doesn't name the report:\n{}", o.stderr);
    let o = run(&["dump", b, "--out", d.join("no_such_folder").join("w.json").to_str().unwrap()]);
    assert!(!o.hung && o.code == Some(2), "dump into a missing folder: {:?}\n{}", o.code, o.stderr);
    let _ = std::fs::remove_dir_all(&d);
}
