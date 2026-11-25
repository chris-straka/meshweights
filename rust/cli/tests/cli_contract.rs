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

fn collect_rs_sources(dir: &std::path::Path, out: &mut String) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_rs_sources(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push_str(&std::fs::read_to_string(&path).unwrap());
            out.push('\n');
        }
    }
}

#[test]
fn help_lists_every_flag() {
    let (c, help, _) = run(&["--help"]);
    assert_eq!(c, 0);
    let mut src = String::new();
    collect_rs_sources(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut src);
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
/// bad case delegated to `gk blind make` / `gk blind score`, the key
/// outside the judged folder, and `blind-score` retiring the old score
/// only where the judge and tears agree against it.
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
    // Mock `gk blind make|score` so single-repo CI runs cleanly while verifying delegation.
    let gk_log = d.join("gk.log");
    let fake_gk = d.join("gk");
    std::fs::write(
        &fake_gk,
        format!(
            r#"#!/usr/bin/env python3
import json, os, shutil, sys
open({:?}, "a").write(" ".join(sys.argv[1:]) + "\n")
assert sys.argv[1] == "blind", sys.argv
sub = sys.argv[2]
if sub == "make":
    a_dir = sys.argv[sys.argv.index("--a") + 1]
    b_dir = sys.argv[sys.argv.index("--b") + 1]
    out_dir = sys.argv[sys.argv.index("--out") + 1]
    planted = sys.argv[sys.argv.index("--planted") + 1]
    seed = int(sys.argv[sys.argv.index("--seed") + 1])
    q = sys.argv[sys.argv.index("--question") + 1]
    os.makedirs(out_dir, exist_ok=True)
    imgs = sorted(f for f in os.listdir(a_dir) if f.endswith(".png"))
    pub, key = [], []
    n = 0
    for name in imgs:
        n += 1
        pid = f"pair-{{n:02d}}"
        shutil.copyfile(os.path.join(a_dir, name), os.path.join(out_dir, f"{{pid}}-left.png"))
        shutil.copyfile(os.path.join(b_dir, name), os.path.join(out_dir, f"{{pid}}-right.png"))
        shutil.copyfile(os.path.join(a_dir, name), os.path.join(out_dir, f"{{pid}}.png"))
        pub.append({{"id": pid, "left": f"{{pid}}-left.png", "right": f"{{pid}}-right.png", "sheet": f"{{pid}}.png"}})
        key.append({{"id": pid, "kind": "claim", "claim": os.path.splitext(name)[0], "left": "a", "rmse": 12.0}})
    n += 1
    pid = f"pair-{{n:02d}}"
    shutil.copyfile(os.path.join(a_dir, imgs[0]), os.path.join(out_dir, f"{{pid}}-left.png"))
    shutil.copyfile(os.path.join(a_dir, imgs[0]), os.path.join(out_dir, f"{{pid}}-right.png"))
    shutil.copyfile(os.path.join(a_dir, imgs[0]), os.path.join(out_dir, f"{{pid}}.png"))
    pub.append({{"id": pid, "left": f"{{pid}}-left.png", "right": f"{{pid}}-right.png", "sheet": f"{{pid}}.png"}})
    key.append({{"id": pid, "kind": "control", "claim": os.path.splitext(imgs[0])[0] + " (control)", "left": "a", "rmse": 0.0}})
    n += 1
    pid = f"pair-{{n:02d}}"
    shutil.copyfile(os.path.join(a_dir, imgs[0]), os.path.join(out_dir, f"{{pid}}-left.png"))
    shutil.copyfile(planted, os.path.join(out_dir, f"{{pid}}-right.png"))
    shutil.copyfile(planted, os.path.join(out_dir, f"{{pid}}.png"))
    pub.append({{"id": pid, "left": f"{{pid}}-left.png", "right": f"{{pid}}-right.png", "sheet": f"{{pid}}.png"}})
    key.append({{"id": pid, "kind": "planted", "claim": os.path.splitext(imgs[0])[0] + " (planted)", "left": "a", "rmse": 45.0}})
    json.dump({{"version": 1, "question": q, "answers": "one of left | right | cant_tell per pair", "pairs": pub}}, open(os.path.join(out_dir, "judge.json"), "w"))
    json.dump({{"version": 1, "seed": seed, "question": q, "pairs": key}}, open(out_dir + ".key.json", "w"))
    sys.exit(0)
if sub == "score":
    out_dir = sys.argv[3]
    ans_path = sys.argv[sys.argv.index("--answers") + 1]
    key_path = sys.argv[sys.argv.index("--key") + 1] if "--key" in sys.argv else out_dir + ".key.json"
    ans = json.load(open(ans_path))
    key = json.load(open(key_path))
    ctrl_ok, plant_ok = True, True
    a_wins, b_wins, cant = 0, 0, 0
    rows = []
    for p in key["pairs"]:
        av = ans.get(p["id"], "cant_tell")
        if p["kind"] == "control":
            ctrl_ok = ctrl_ok and (av == "cant_tell")
            w = "cant_tell"
        elif p["kind"] == "planted":
            want = "left" if p["left"] == "a" else "right"
            plant_ok = plant_ok and (av == want)
            w = "a" if av == want else "b"
        else:
            w = p["left"] if av == "left" else ("b" if p["left"] == "a" and av == "right" else ("a" if av == "right" else "cant_tell"))
            if w == "a": a_wins += 1
            elif w == "b": b_wins += 1
            else: cant += 1
        rows.append({{"pair": p["id"], "kind": p["kind"], "claim": p["claim"], "answer": av, "winner": w}})
    v = {{"version": 1, "pass": ctrl_ok and plant_ok, "control_ok": ctrl_ok, "planted_ok": plant_ok, "claims": {{"a_wins": a_wins, "b_wins": b_wins, "cant_tell": cant}}, "pairs": rows}}
    json.dump(v, open(os.path.join(out_dir, "verdict.json"), "w"))
    print(json.dumps(v))
    sys.exit(0 if v["pass"] else 1)
sys.exit(2)
"#,
            gk_log.display()
        ),
    )
    .unwrap();
    Command::new("chmod").arg("+x").arg(&fake_gk).status().unwrap();

    // Missing gk fails loudly with exit 2.
    let o_missing_gk = Command::new(bin())
        .args([
            "blind",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "--out",
            d.join("no_gk").to_str().unwrap(),
            "--metrics",
            metrics.to_str().unwrap(),
            "--top",
            "2",
        ])
        .env("GK_BIN", d.join("missing_gk"))
        .output()
        .unwrap();
    assert_eq!(
        o_missing_gk.status.code(),
        Some(2),
        "expected exit 2 when GK_BIN missing: {}",
        String::from_utf8_lossy(&o_missing_gk.stderr)
    );
    assert!(String::from_utf8_lossy(&o_missing_gk.stderr).to_lowercase().contains("gk"));

    let o = Command::new(bin())
        .args([
            "blind",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--metrics",
            metrics.to_str().unwrap(),
            "--top",
            "4",
        ])
        .env("GK_BIN", &fake_gk)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout);
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(std::fs::read_to_string(&gk_log).unwrap().contains("blind make"));

    let pubdoc: serde_json::Value = serde_json::from_slice(&std::fs::read(out.join("judge.json")).unwrap()).unwrap();
    let pairs = pubdoc["pairs"].as_array().unwrap();
    assert!(pairs.len() >= 4, "{pairs:?}");
    for p in pairs {
        assert_eq!(&std::fs::read(out.join(p["sheet"].as_str().unwrap())).unwrap()[1..4], b"PNG");
        assert!(p["left"].as_str().unwrap().starts_with("pair-"));
    }
    assert!(!out.join("answer_key.json").exists() && !out.join("pairs.key.json").exists());
    let key: serde_json::Value = serde_json::from_slice(&std::fs::read(d.join("pairs.key.json")).unwrap()).unwrap();
    let kpairs = key["pairs"].as_array().unwrap();
    assert_eq!(kpairs.len(), pairs.len());
    assert_eq!(kpairs.iter().filter(|p| p["category"] == "control").count(), 1);
    assert_eq!(kpairs.iter().filter(|p| p["category"] == "planted").count(), 1);
    assert!(kpairs.iter().filter(|p| p["category"] == "claim").all(|p| p["tears_winner"] == "A" || p["tears_winner"] == "B"));

    // A perfect judge: tears' winner on claims, can't-tell on the control, unplanted rig on planted.
    let mut answers = serde_json::Map::new();
    for p in kpairs {
        let id = p["id"].as_str().unwrap();
        let ans = match p["category"].as_str().unwrap() {
            "control" => "cant_tell",
            _ => {
                let want = p["expected"].as_str().unwrap();
                if p["left_rig"].as_str().unwrap() == want { "left" } else { "right" }
            }
        };
        answers.insert(id.into(), ans.into());
    }
    let af = d.join("answers.json");
    std::fs::write(&af, serde_json::to_string(&answers).unwrap()).unwrap();
    let os = Command::new(bin())
        .args(["blind-score", out.to_str().unwrap(), "--answers", af.to_str().unwrap()])
        .env("GK_BIN", &fake_gk)
        .output()
        .unwrap();
    assert_eq!(os.status.code(), Some(0), "{}\n{}", String::from_utf8_lossy(&os.stdout), String::from_utf8_lossy(&os.stderr));
    assert!(std::fs::read_to_string(&gk_log).unwrap().contains("blind score"));
    let verdict: serde_json::Value = serde_json::from_slice(&std::fs::read(out.join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["pass"], true, "{verdict}");
    assert_eq!(verdict["control_ok"], true);
    assert_eq!(verdict["planted_ok"], true);
    assert_eq!(verdict["planted_tears_ok"], true);
    let retired = verdict["score_retired_pairs"].as_array().unwrap();
    assert!(!retired.is_empty(), "{verdict}");
    // Picking the planted side (also using `pair_NN` underscore alias) must fail the run.
    let mut bad_answers = serde_json::Map::new();
    for p in kpairs {
        let n = p["pair"].as_u64().unwrap();
        let ans = if p["category"] == "planted" {
            if p["left_rig"].as_str().unwrap() == p["expected"].as_str().unwrap() { "right" } else { "left" }
        } else {
            answers[p["id"].as_str().unwrap()].as_str().unwrap()
        };
        bad_answers.insert(format!("pair_{n:02}"), ans.into());
    }
    std::fs::write(&af, serde_json::to_string(&bad_answers).unwrap()).unwrap();
    let ob = Command::new(bin())
        .args(["blind-score", out.to_str().unwrap(), "--answers", af.to_str().unwrap()])
        .env("GK_BIN", &fake_gk)
        .output()
        .unwrap();
    assert_eq!(ob.status.code(), Some(1), "{}", String::from_utf8_lossy(&ob.stdout));
    // Unknown flags are usage errors here too.
    assert_eq!(run(&["blind", a.to_str().unwrap(), b.to_str().unwrap(), "--out", out.to_str().unwrap(), "--seed", "x"]).0, 2);
    assert_eq!(run(&["blind-score", out.to_str().unwrap()]).0, 2);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn blind_follow_requires_explicit_metrics_without_home_fallback() {
    let d = tmp("blind_no_fallback");
    let ds = d.to_str().unwrap();
    assert_eq!(run(&["fixture", "all", "--out", ds]).0, 0);
    let a = d.join("mannequin_bleed.glb");
    let b = d.join("mannequin_cape_wrong_bone.glb");
    // Even if $HOME contains an example metrics.ron path, follow blind must
    // refuse unless --metrics (or METRICS_FILE) is explicitly provided.
    let fake_home = d.join("home");
    let fake_metrics = fake_home.join("game/assets/data/metrics.ron");
    std::fs::create_dir_all(fake_metrics.parent().unwrap()).unwrap();
    std::fs::write(&fake_metrics, r#"{"camera": {"distance": 3.0, "focus": 1.535, "fov": 75.0}}"#).unwrap();

    let fake_gk = d.join("gk");
    std::fs::write(
        &fake_gk,
        "#!/usr/bin/env python3\nimport json, os, shutil, sys\nout = sys.argv[sys.argv.index('--out')+1]\na = sys.argv[sys.argv.index('--a')+1]\nb = sys.argv[sys.argv.index('--b')+1]\nos.makedirs(out, exist_ok=True)\nimgs = sorted(os.listdir(a))\njson.dump({'version':1,'question':'q','answers':'x','pairs':[{'id':'pair-01','left':'pair-01-left.png','right':'pair-01-right.png','sheet':'pair-01.png'},{'id':'pair-02','left':'pair-02-left.png','right':'pair-02-right.png','sheet':'pair-02.png'},{'id':'pair-03','left':'pair-03-left.png','right':'pair-03-right.png','sheet':'pair-03.png'}]}, open(os.path.join(out,'judge.json'),'w'))\njson.dump({'version':1,'seed':1,'question':'q','pairs':[{'id':'pair-01','kind':'claim','claim':imgs[0][:-4],'left':'a','rmse':10.0},{'id':'pair-02','kind':'control','claim':'ctrl','left':'a','rmse':0.0},{'id':'pair-03','kind':'planted','claim':'pl','left':'a','rmse':50.0}]}, open(out+'.key.json','w'))\n",
    )
    .unwrap();
    Command::new("chmod").arg("+x").arg(&fake_gk).status().unwrap();

    let out_missing = d.join("pairs_missing");
    let o = Command::new(bin())
        .args(["blind", a.to_str().unwrap(), b.to_str().unwrap(), "--out", out_missing.to_str().unwrap(), "--top", "1"])
        .env("HOME", &fake_home)
        .env("GK_BIN", &fake_gk)
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
        .env("GK_BIN", &fake_gk)
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

#[test]
fn blind_and_blind_score_surface_gk_stdout_errors() {
    let d = tmp("gk_stdout_err");
    let ds = d.to_str().unwrap();
    assert_eq!(run(&["fixture", "all", "--out", ds]).0, 0);
    let a = d.join("mannequin_bleed.glb");
    let b = d.join("mannequin_cape_wrong_bone.glb");
    let metrics = d.join("metrics.ron");
    std::fs::write(&metrics, r#"{"camera": {"distance": 3.0, "focus": 1.535, "fov": 75.0}}"#).unwrap();

    // Real `gk blind make` / `gk blind score` write errors to stdout (`println!`)
    // and leave stderr empty. `weights` must surface stdout instead of printing `: `.
    let fake_gk = d.join("gk");
    std::fs::write(
        &fake_gk,
        r#"#!/usr/bin/env python3
import sys
sub = sys.argv[2]
if sub == "make":
    print("gk blind: the planted bad case barely differs from the good picture (RMSE 3.42); a judge cannot be failed by it")
    sys.exit(1)
if sub == "score":
    print("gk blind: control pair-02 is not the same picture twice (RMSE 1.00); the set is broken")
    sys.exit(2)
sys.exit(2)
"#,
    )
    .unwrap();
    Command::new("chmod").arg("+x").arg(&fake_gk).status().unwrap();

    let o_make = Command::new(bin())
        .args([
            "blind",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "--out",
            d.join("out").to_str().unwrap(),
            "--metrics",
            metrics.to_str().unwrap(),
            "--top",
            "1",
        ])
        .env("GK_BIN", &fake_gk)
        .output()
        .unwrap();
    assert_eq!(o_make.status.code(), Some(2));
    let err_make = String::from_utf8_lossy(&o_make.stderr);
    assert!(err_make.contains("barely differs") && err_make.contains("RMSE 3.42"), "dropped stdout error: {err_make:?}");
    assert!(!err_make.trim_end().ends_with(':'), "trailing empty colon in: {err_make:?}");

    let pairs_dir = d.join("pairs");
    std::fs::create_dir_all(&pairs_dir).unwrap();
    let key_path = d.join("pairs.key.json");
    std::fs::write(
        &key_path,
        serde_json::json!({
            "version": 1,
            "rig_a": "a.glb",
            "rig_b": "b.glb",
            "section": "poses",
            "seed": 1,
            "planted": "swap",
            "tears_pct": {"A": 0.0, "B": 1.0},
            "scores": {"A": 90.0, "B": 50.0},
            "pairs": [
                {"pair": 1, "category": "control", "focus": "body", "frame": "p0", "left_rig": "A", "right_rig": "A", "expected": "cant_tell", "tears_cm2": {"A": 0.0}, "tears_winner": "tie", "score_winner": "tie"},
                {"pair": 2, "category": "planted", "focus": "body", "frame": "p0", "left_rig": "A", "right_rig": "PLANTED_A", "expected": "A", "tears_cm2": {"A": 0.0, "PLANTED_A": 100.0}, "tears_winner": "A", "score_winner": "A"}
            ]
        })
        .to_string(),
    )
    .unwrap();
    let answers_path = d.join("answers.json");
    std::fs::write(&answers_path, r#"{"pair-01": "cant_tell", "pair-02": "left"}"#).unwrap();

    let o_score = Command::new(bin())
        .args([
            "blind-score",
            pairs_dir.to_str().unwrap(),
            "--answers",
            answers_path.to_str().unwrap(),
            "--key",
            key_path.to_str().unwrap(),
        ])
        .env("GK_BIN", &fake_gk)
        .output()
        .unwrap();
    assert_eq!(o_score.status.code(), Some(2));
    let err_score = String::from_utf8_lossy(&o_score.stderr);
    assert!(err_score.contains("the set is broken"), "dropped stdout error: {err_score:?}");
    assert!(!err_score.trim_end().ends_with(':'), "trailing empty colon in: {err_score:?}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn planted_stays_one_role_on_an_input_frame_and_gait_flag_runs() {
    let d = tmp("sym_blind_and_gait");
    let ds = d.to_str().unwrap();
    assert_eq!(run(&["fixture", "all", "--out", ds]).0, 0);
    let clean = d.join("mannequin_clean.glb");
    let bleed = d.join("mannequin_bleed.glb");
    let metrics = d.join("metrics.ron");
    std::fs::write(&metrics, r#"{"camera": {"distance": 3.0, "fov": 75.0}}"#).unwrap();

    // Purely symmetric ROM poses: the planted case must still be shown on
    // this input frame, never on a stride made up for it.
    let sym_poses = d.join("sym.ron");
    std::fs::write(
        &sym_poses,
        r#"PoseSet(name: "sym_only", poses: [
            PoseSpec(name: "both_arms_fwd", moves: [
                Move(bone: "upper_arm.L", motion: Bend(Forward, 45)),
                Move(bone: "upper_arm.R", motion: Bend(Forward, 45)),
            ]),
        ])"#,
    )
    .unwrap();

    // Verify --gait flag is accepted cleanly by check.
    let (c_gait, out_gait, err_gait) = run(&["check", clean.to_str().unwrap(), "--gait", "--json"]);
    assert_eq!(c_gait, 0, "check --gait failed: {err_gait}");
    let vj: serde_json::Value = serde_json::from_str(&out_gait).unwrap();
    assert_eq!(vj["class"], "humanoid");

    // A minimal fake gk that records the planted picture's RMSE without judging it.
    let fake_gk = d.join("gk");
    let rmse_log = d.join("rmse.txt");
    std::fs::write(
        &fake_gk,
        format!(
            r#"#!/usr/bin/env python3
import json, math, os, shutil, struct, sys, zlib
def read_png(p):
    d = open(p, 'rb').read(); i = 8; w = h = 0; idat = bytearray()
    while i < len(d):
        ln, tp = struct.unpack('>I4s', d[i:i+8])
        c = d[i+8:i+8+ln]; i += 12 + ln
        if tp == b'IHDR': w, h = struct.unpack('>II', c[:8])
        elif tp == b'IDAT': idat.extend(c)
        elif tp == b'IEND': break
    raw = zlib.decompress(bytes(idat)); st = w * 3; out = bytearray(); prev = bytearray(st)
    for y in range(h):
        f = raw[y*(st+1)]; row = bytearray(raw[y*(st+1)+1:(y+1)*(st+1)])
        if f == 1:
            for k in range(3, st): row[k] = (row[k] + row[k-3]) & 0xFF
        elif f == 2:
            for k in range(st): row[k] = (row[k] + prev[k]) & 0xFF
        elif f == 3:
            for k in range(st): row[k] = (row[k] + ((row[k-3] if k>=3 else 0)+prev[k])//2) & 0xFF
        elif f == 4:
            for k in range(st):
                a = row[k-3] if k>=3 else 0; b = prev[k]; c = prev[k-3] if k>=3 else 0
                p2 = a + b - c; pa, pb, pc = abs(p2-a), abs(p2-b), abs(p2-c)
                row[k] = (row[k] + (a if pa<=pb and pa<=pc else (b if pb<=pc else c))) & 0xFF
        out.extend(row); prev = row
    return out
a_dir = sys.argv[sys.argv.index('--a')+1]; b_dir = sys.argv[sys.argv.index('--b')+1]
out_dir = sys.argv[sys.argv.index('--out')+1]; pl = sys.argv[sys.argv.index('--planted')+1]
imgs = sorted(f for f in os.listdir(a_dir) if f.endswith('.png'))
pa = read_png(os.path.join(a_dir, imgs[0])); pp = read_png(pl)
pr = math.sqrt(sum((x-y)*(x-y) for x, y in zip(pa, pp)) / len(pa))
open({:?}, 'w').write(f'{{pr:.4f}}\n')
os.makedirs(out_dir, exist_ok=True)
pairs = [{{'id':'pair-01','kind':'control','claim':os.path.splitext(imgs[0])[0],'left':'a','rmse':0.0}},
         {{'id':'pair-02','kind':'planted','claim':os.path.splitext(imgs[0])[0],'left':'a','rmse':pr}}]
json.dump({{'version':1,'pairs':[]}}, open(os.path.join(out_dir, 'judge.json'), 'w'))
json.dump({{'version':1,'pairs':pairs}}, open(out_dir + '.key.json', 'w'))
"#,
            rmse_log.display()
        ),
    )
    .unwrap();
    Command::new("chmod").arg("+x").arg(&fake_gk).status().unwrap();

    let out = d.join("sym_blind");
    let o = Command::new(bin())
        .args([
            "blind",
            bleed.to_str().unwrap(),
            clean.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--poses",
            sym_poses.to_str().unwrap(),
            "--no-clips",
            "--camera",
            "follow",
            "--metrics",
            metrics.to_str().unwrap(),
        ])
        .env("GK_BIN", &fake_gk)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let key: serde_json::Value = serde_json::from_slice(&std::fs::read(d.join("sym_blind.key.json")).unwrap()).unwrap();
    let planted_pair = key["pairs"].as_array().unwrap().iter().find(|p| p["category"] == "planted").unwrap();
    assert_eq!(planted_pair["frame"], "both_arms_fwd", "planted must sit on an input frame: {planted_pair}");
    // One mirrored role pair, not a whole limb chain with its helpers.
    assert_eq!(key["planted"], "the left and right upper_arm weights were swapped", "{}", key["planted"]);
    let _ = std::fs::remove_dir_all(&d);
}
