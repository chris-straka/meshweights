//! CLI contract: exit codes, outputs, and that --help lists every flag.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_weights"))
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wf_cli_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(args: &[&str]) -> (i32, String, String) {
    let o = Command::new(bin()).args(args).output().unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned())
}

#[test]
fn exit_codes_and_outputs() {
    let d = tmp("contract");
    let ds = d.to_str().unwrap();
    assert_eq!(run(&["fixture", "all", "--out", ds]).0, 0);
    let clean = d.join("mannequin_clean.glb");
    let bleed = d.join("mannequin_bleed.glb");
    let (c, out, _) = run(&["check", clean.to_str().unwrap()]);
    assert_eq!(c, 0, "{out}");
    // The headline first (tears on the poses), the older verdict after.
    assert!(out.starts_with("mannequin_clean.glb") && out.contains("poses (") && out.contains("torn"), "{out}");
    assert!(out.lines().any(|l| l.starts_with("PASS")), "{out}");
    let (c, out, _) = run(&["check", bleed.to_str().unwrap(), "--json"]);
    assert_eq!(c, 1);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["pass"], false);
    assert!(v["findings"].as_array().unwrap().iter().any(|f| f["code"] == "D_BLEED"));

    let fixed = d.join("fixed.glb");
    let sheet = d.join("ab.png");
    let (c, out, _) = run(&["fix", bleed.to_str().unwrap(), "--out", fixed.to_str().unwrap(), "--sheet", sheet.to_str().unwrap()]);
    assert_eq!(c, 0, "{out}");
    assert!(out.starts_with("FIXED"));
    assert!(fixed.exists() && sheet.exists() && d.join("fixed.report.json").exists());
    assert_eq!(run(&["check", fixed.to_str().unwrap()]).0, 0);

    let dump = d.join("w.json");
    assert_eq!(run(&["dump", fixed.to_str().unwrap(), "--out", dump.to_str().unwrap()]).0, 0);
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&dump).unwrap()).unwrap();
    assert_eq!(v["joints"].as_array().unwrap().len(), 20);

    let png = d.join("s.png");
    assert_eq!(run(&["sheet", bleed.to_str().unwrap(), "--out", png.to_str().unwrap()]).0, 1);
    assert_eq!(&std::fs::read(&png).unwrap()[1..4], b"PNG");
    let cmp = d.join("c.png");
    assert_eq!(run(&["compare", bleed.to_str().unwrap(), fixed.to_str().unwrap(), "--out", cmp.to_str().unwrap()]).0, 0);
    let zoom = d.join("z.png");
    let args = ["compare", bleed.to_str().unwrap(), fixed.to_str().unwrap(), "--out", zoom.to_str().unwrap()];
    assert_eq!(run(&[&args[..], &["--zoom", "--rows", "4", "--label-a", "before", "--label-b", "after"]].concat()).0, 0);
    assert_eq!(&std::fs::read(&zoom).unwrap()[1..4], b"PNG");
    assert_eq!(run(&[&args[..], &["--zoom", "--rows", "0"]].concat()).0, 2);
    // Clips: an mp4 a row where ffmpeg is installed, else the frames.
    let clips = d.join("clips");
    assert_eq!(run(&[&args[..], &["--zoom", "--rows", "1", "--clips", clips.to_str().unwrap()]].concat()).0, 0);
    let made: Vec<String> = std::fs::read_dir(&clips).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(made.iter().any(|n| n.starts_with("01_")), "{made:?}");
    assert_eq!(run(&[&args[..], &["--clips", clips.to_str().unwrap()]].concat()).0, 2);
    // Probe: numbered, unmarked loops and one JSON line a case.
    let probe = d.join("probe");
    let (code, out, _) = run(&["probe", bleed.to_str().unwrap(), "--out", probe.to_str().unwrap(), "--number-from", "7"]);
    assert_eq!(code, 0, "{out}");
    let cases: Vec<serde_json::Value> = out.lines().filter(|l| l.starts_with('{')).map(|l| serde_json::from_str(l).unwrap()).collect();
    assert!(!cases.is_empty(), "{out}");
    assert_eq!(cases[0]["case"], 7);
    assert!(cases.iter().all(|c| c["stretch"].as_f64().unwrap() > 1.0 && std::path::Path::new(c["file"].as_str().unwrap()).exists()));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn usage_errors_exit_2() {
    assert_eq!(run(&[]).0, 2);
    assert_eq!(run(&["frobnicate"]).0, 2);
    assert_eq!(run(&["check"]).0, 2);
    assert_eq!(run(&["check", "/nonexistent.glb"]).0, 2);
    assert_eq!(run(&["fix", "x.glb"]).0, 2);
    assert_eq!(run(&["fix", "x.glb", "--out", "y.glb", "--method", "magic"]).0, 2);
    assert_eq!(run(&["check", "x.glb", "--voxels", "3"]).0, 2);
}

#[test]
fn help_lists_every_flag() {
    let (c, help, _) = run(&["--help"]);
    assert_eq!(c, 0);
    let src = include_str!("../src/main.rs");
    let mut flags: Vec<&str> = src.split('"').filter(|s| s.starts_with("--") && s.len() > 2 && !s.contains(' ')).collect();
    flags.sort();
    flags.dedup();
    for f in flags {
        assert!(help.contains(f), "--help does not mention {f}");
    }
}

#[test]
fn skintokens_candidate_is_scored() {
    let d = tmp("skintokens");
    let ds = d.to_str().unwrap();
    assert_eq!(run(&["fixture", "all", "--out", ds]).0, 0);
    let bleed = d.join("mannequin_bleed.glb");
    let clean = d.join("mannequin_clean.glb");
    // Fake `skintokens skin IN OUT --report R`: answers with the clean twin's weights.
    let fake = d.join("skintokens");
    std::fs::write(&fake, format!("#!/bin/sh\ncp '{}' \"$3\"\necho '{{\"ok\": true}}' > \"$5\"\n", clean.display())).unwrap();
    std::process::Command::new("chmod").arg("+x").arg(&fake).status().unwrap();
    let fixed = d.join("fixed.glb");
    let o = Command::new(bin())
        .args(["fix", bleed.to_str().unwrap(), "--out", fixed.to_str().unwrap(), "--skintokens"])
        .env("SKINTOKENS_BIN", &fake)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let rep = std::fs::read_to_string(d.join("fixed.report.json")).unwrap();
    assert!(rep.contains("external:skintokens"), "{rep}");
    // Missing tool: usage error, nothing written.
    let o = Command::new(bin())
        .args(["fix", bleed.to_str().unwrap(), "--out", d.join("x.glb").to_str().unwrap(), "--skintokens"])
        .env("SKINTOKENS_BIN", d.join("missing"))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(!d.join("x.glb").exists());
    let _ = std::fs::remove_dir_all(&d);
}

/// Blind pairs: one per claim plus a can't-tell control and a planted
/// bad case, the key outside the judged folder, and `blind-score`
/// retiring the old score only where the judge and tears agree
/// against it. The test reads the key (it is not the judge) and
/// answers as a perfect judge would.
#[test]
fn blind_pairs_and_scoring() {
    let d = tmp("blind");
    let ds = d.to_str().unwrap();
    assert_eq!(run(&["fixture", "all", "--out", ds]).0, 0);
    // Bleed (score 9.7) against the cape on the wrong bone (18.1): the
    // score ranks the cape rig better, tears rank the bleed rig
    // better on the cape's claims — the disagreement the score loses.
    let a = d.join("mannequin_bleed.glb");
    let b = d.join("mannequin_cape_wrong_bone.glb");
    let out = d.join("pairs");
    let metrics = d.join("metrics.ron");
    std::fs::write(&metrics, r#"{"camera": {"distance": 3.0, "focus": 1.535, "fov": 75.0}}"#).unwrap();
    let (c, stdout, _) = run(&[
        "blind",
        a.to_str().unwrap(),
        b.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--metrics",
        metrics.to_str().unwrap(),
        "--top",
        "4",
    ]);
    assert_eq!(c, 0, "{stdout}");
    let pubdoc: serde_json::Value = serde_json::from_slice(&std::fs::read(out.join("pairs.json")).unwrap()).unwrap();
    let pairs = pubdoc["pairs"].as_array().unwrap();
    assert!(pairs.len() >= 4, "{pairs:?}");
    for p in pairs {
        assert_eq!(&std::fs::read(out.join(p["file"].as_str().unwrap())).unwrap()[1..4], b"PNG");
        // The judged folder names no rig and holds no key.
        assert!(p["left"].as_str().unwrap().starts_with("pair_"));
    }
    assert!(!out.join("answer_key.json").exists() && !out.to_string_lossy().contains("key"));
    let key: serde_json::Value = serde_json::from_slice(&std::fs::read(d.join("pairs.key.json")).unwrap()).unwrap();
    let kpairs = key["pairs"].as_array().unwrap();
    assert_eq!(kpairs.len(), pairs.len());
    assert_eq!(kpairs.iter().filter(|p| p["category"] == "control").count(), 1);
    assert_eq!(kpairs.iter().filter(|p| p["category"] == "planted").count(), 1);
    assert!(kpairs.iter().filter(|p| p["category"] == "claim").all(|p| p["tears_winner"] == "A" || p["tears_winner"] == "B"));
    // A perfect judge: tears' winner on claims, can't-tell on the
    // control, the unplanted rig on the planted case.
    let mut answers = serde_json::Map::new();
    for p in kpairs {
        let n = p["pair"].as_u64().unwrap();
        let ans = match p["category"].as_str().unwrap() {
            "control" => "cant_tell",
            _ => {
                let want = p["expected"].as_str().unwrap();
                if p["left_rig"].as_str().unwrap() == want { "left" } else { "right" }
            }
        };
        answers.insert(format!("pair_{n:02}"), ans.into());
    }
    let af = d.join("answers.json");
    std::fs::write(&af, serde_json::to_string(&answers).unwrap()).unwrap();
    let (c, stdout, _) = run(&["blind-score", out.to_str().unwrap(), "--answers", af.to_str().unwrap()]);
    assert_eq!(c, 0, "{stdout}");
    let verdict: serde_json::Value = serde_json::from_slice(&std::fs::read(out.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["pass"], true, "{verdict}");
    assert_eq!(verdict["control_ok"], true);
    assert_eq!(verdict["planted_ok"], true);
    assert_eq!(verdict["planted_tears_ok"], true);
    // On the cape's claims the judge and tears agree against the
    // score (18.1 beats 9.7 there): those pairs retire it.
    let retired = verdict["score_retired_pairs"].as_array().unwrap();
    assert!(!retired.is_empty(), "{verdict}");
    // Picking the planted side must fail the run.
    for p in kpairs.iter().filter(|p| p["category"] == "planted") {
        let n = p["pair"].as_u64().unwrap();
        let bad_side = if p["left_rig"].as_str().unwrap() == p["expected"].as_str().unwrap() { "right" } else { "left" };
        answers.insert(format!("pair_{n:02}"), bad_side.into());
    }
    std::fs::write(&af, serde_json::to_string(&answers).unwrap()).unwrap();
    let (c, stdout, _) = run(&["blind-score", out.to_str().unwrap(), "--answers", af.to_str().unwrap()]);
    assert_eq!(c, 1, "{stdout}");
    // Unknown flags are usage errors here too.
    assert_eq!(run(&["blind", a.to_str().unwrap(), b.to_str().unwrap(), "--out", out.to_str().unwrap(), "--seed", "x"]).0, 2);
    assert_eq!(run(&["blind-score", out.to_str().unwrap()]).0, 2);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn blind_follow_requires_explicit_metrics_without_hlh_fallback() {
    let d = tmp("blind_no_fallback");
    let ds = d.to_str().unwrap();
    assert_eq!(run(&["fixture", "all", "--out", ds]).0, 0);
    let a = d.join("mannequin_bleed.glb");
    let b = d.join("mannequin_cape_wrong_bone.glb");
    // Even if $HOME contains Games/hlh/bevy/assets/data/metrics.ron, follow blind
    // must refuse unless --metrics (or HLH_METRICS/METRICS_FILE) is explicitly provided.
    let fake_home = d.join("home");
    let fake_hlh_metrics = fake_home.join("Games/hlh/bevy/assets/data/metrics.ron");
    std::fs::create_dir_all(fake_hlh_metrics.parent().unwrap()).unwrap();
    std::fs::write(&fake_hlh_metrics, r#"{"camera": {"distance": 3.0, "focus": 1.535, "fov": 75.0}}"#).unwrap();

    let out_missing = d.join("pairs_missing");
    let o = Command::new(bin())
        .args(["blind", a.to_str().unwrap(), b.to_str().unwrap(), "--out", out_missing.to_str().unwrap(), "--top", "1"])
        .env("HOME", &fake_home)
        .env_remove("HLH_METRICS")
        .env_remove("METRICS_FILE")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2), "expected exit 2 without --metrics; stderr:\n{}", String::from_utf8_lossy(&o.stderr));

    // Explicit --metrics <path> must succeed with no env vars set.
    let explicit_metrics = d.join("cam_metrics.ron");
    std::fs::write(&explicit_metrics, r#"{"camera": {"distance": 3.0, "focus": 1.535, "fov": 75.0}}"#).unwrap();
    let out_ok = d.join("pairs_ok");
    let o2 = Command::new(bin())
        .args([
            "blind",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "--out",
            out_ok.to_str().unwrap(),
            "--metrics",
            explicit_metrics.to_str().unwrap(),
            "--top",
            "1",
        ])
        .env("HOME", &fake_home)
        .env_remove("HLH_METRICS")
        .env_remove("METRICS_FILE")
        .output()
        .unwrap();
    assert_eq!(o2.status.code(), Some(0), "explicit --metrics failed:\n{}", String::from_utf8_lossy(&o2.stderr));
    let _ = std::fs::remove_dir_all(&d);
}

/// A mistyped or misplaced flag is a usage error, not silently ignored
/// (`--jsn` used to print the text report and exit 0).
#[test]
fn unknown_flags_are_an_error() {
    let d = tmp("flags");
    let ds = d.to_str().unwrap();
    assert_eq!(run(&["fixture", "clean", "--out", ds]).0, 0);
    let clean = d.join("mannequin_clean.glb");
    let c = clean.to_str().unwrap();
    let (code, _, err) = run(&["check", c, "--jsn"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("--jsn"), "{err}");
    // A flag of another command is unknown here too.
    assert_eq!(run(&["check", c, "--zoom"]).0, 2);
    assert_eq!(run(&["fix", c, "--out", d.join("f.glb").to_str().unwrap(), "--rows", "3"]).0, 2);
    assert!(!d.join("f.glb").exists());
    // Known ones still work.
    assert_eq!(run(&["check", c, "--json", "--no-clips"]).0, 0);
    let _ = std::fs::remove_dir_all(&d);
}
