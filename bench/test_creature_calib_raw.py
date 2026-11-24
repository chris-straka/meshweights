#!/usr/bin/env python3
"""Tests for raw pre-fix creature benchmark enforcement in bench/creature_calib.py."""

import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCRIPT = os.path.join(REPO, "bench", "creature_calib.py")


def make_stub_weights(path: str, spec: dict) -> None:
    spec_json = json.dumps(spec)
    code = f"""#!/usr/bin/env python3
import json, os, shutil, sys

SPEC = json.loads({spec_json!r})
cmd = sys.argv[1]
log_path = SPEC.get("log_path")
if log_path:
    with open(log_path, "a") as lf:
        lf.write(json.dumps(sys.argv[1:]) + "\\n")

def stem(p):
    base = os.path.basename(p)
    if base in ("raw.glb", "truth.glb", "unfixed.glb"):
        return os.path.basename(os.path.dirname(p))
    return os.path.splitext(base)[0]

if cmd == "check":
    inp = sys.argv[2]
    key = stem(inp)
    entry = SPEC.get("check", {{}}).get(key, {{"poses": 0.4, "clips": 0.3, "score": 90.0}})
    headline = []
    if entry.get("clips") is not None:
        headline.append({{
            "name": "game clips",
            "frames": 16,
            "clips": ["walk", "trot"],
            "source": "own",
            "tears_pct": entry["clips"],
            "tears_verts_pct": entry["clips"],
            "through_cm2": 0.0,
            "findings": [],
        }})
    headline.append({{
        "name": "poses",
        "frames": 12,
        "clips": [],
        "source": "range of motion",
        "tears_pct": entry["poses"],
        "tears_verts_pct": entry["poses"],
        "through_cm2": 0.0,
        "findings": [],
    }})
    print(json.dumps({{
        "file": os.path.basename(inp),
        "class": "quadruped",
        "score": entry.get("score", 85.0),
        "pass": entry["poses"] < 1.0,
        "headline": headline,
        "findings": [],
    }}))
    sys.exit(0)

if cmd == "fix":
    inp = sys.argv[2]
    out = sys.argv[sys.argv.index("--out") + 1]
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    shutil.copyfile(inp, out)
    key = stem(inp)
    entry = SPEC.get("fix", {{}}).get(key, {{}})
    rep = {{
        "before": {{"score": 50.0, "pass": False, "fails": 2}},
        "after": {{"score": 78.0, "pass": False, "fails": 1}},
        "improved": entry.get("improved", True),
        "verts_changed": entry.get("verts_changed", 120),
        "mean_l1_change": entry.get("mean_l1_change", 0.08),
    }}
    with open(os.path.splitext(out)[0] + ".report.json", "w") as f:
        json.dump(rep, f)
    sys.exit(0)

if cmd == "blind":
    inp = sys.argv[2]
    out = sys.argv[sys.argv.index("--out") + 1]
    os.makedirs(out, exist_ok=True)
    key = stem(inp)
    claims = SPEC.get("claims", {{}}).get(key, [
        {{"category": "claim", "expected": "A", "rmse": 6.4, "frame": "trot@0.20s"}},
        {{"category": "control", "expected": "cant_tell", "rmse": 0.0, "frame": "walk@0.33s"}},
        {{"category": "planted", "expected": "A", "rmse": 18.2, "frame": "trot@0.20s"}},
    ])
    key_path = os.path.join(os.path.dirname(out), os.path.basename(out) + ".key.json")
    with open(key_path, "w") as f:
        json.dump({{"section": "game clips", "tears_pct": {{"A": 4.0, "B": 1.2}}, "pairs": claims}}, f)
    sys.exit(0)

if cmd == "blind-score":
    bdir = sys.argv[2]
    v = {{"pass": True, "control_ok": True, "planted_ok": True, "planted_tears_ok": True, "claims_judged": 2, "judge_agrees_tears": 2}}
    with open(os.path.join(bdir, "verdict.json"), "w") as f:
        json.dump(v, f)
    sys.exit(0)

sys.exit(2)
"""
    with open(path, "w", encoding="utf-8") as f:
        f.write(code)
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)


def run_calib(*args):
    return subprocess.run([sys.executable, SCRIPT, *args], capture_output=True, text=True)


def test_raw_subdir_and_require_raw():
    with tempfile.TemporaryDirectory() as td:
        art = os.path.join(td, "artists")
        gen = os.path.join(td, "generated")
        os.makedirs(os.path.join(art, "beast_a"))
        os.makedirs(os.path.join(gen, "raw_01"))
        open(os.path.join(art, "beast_a", "truth.glb"), "wb").write(b"a")
        open(os.path.join(gen, "raw_01", "raw.glb"), "wb").write(b"g")

        log_path = os.path.join(td, "invocations.jsonl")
        spec = {
            "log_path": log_path,
            "check": {
                "beast_a": {"poses": 0.3, "clips": 0.2, "score": 94.0},
                "raw_01": {"poses": 5.5, "clips": 3.4, "score": 55.0},
                "raw_01_fixed": {"poses": 1.4, "clips": 0.9, "score": 82.0},
            },
            "fix": {"raw_01": {"verts_changed": 340, "mean_l1_change": 0.12, "improved": True}},
        }
        stub = os.path.join(td, "weights_stub.py")
        make_stub_weights(stub, spec)
        out = os.path.join(td, "out")
        res = run_calib("--weights", stub, "--artists-dir", art, "--generated-dir", gen, "--out", out, "--require-raw", "--gait")
        assert res.returncode == 0, f"raw calibration failed: {res.stdout} {res.stderr}"

        calib = json.load(open(os.path.join(out, "calib.json")))
        assert calib["raw_ok"] is True
        assert calib["require_raw"] is True
        g = calib["generated"][0]
        assert g["already_fixed"] is False
        assert g["verts_changed"] == 340
        assert g["distinct_claims"] == 1
        assert g["planted_frame"] == "trot@0.20s"

        invocations = [json.loads(line) for line in open(log_path)]
        assert any(args[0] == "check" and "--gait" in args for args in invocations)
        assert any(args[0] == "blind" and "--gait" in args for args in invocations)

        ans_dir = os.path.join(out, "blind", "raw_01")
        with open(os.path.join(ans_dir, "answers.json"), "w") as f:
            json.dump({"answers": {"1": "left"}}, f)
        rs = run_calib("--weights", stub, "--score", out)
        assert rs.returncode == 0, f"score failed: {rs.stdout} {rs.stderr}"
        verdict = json.load(open(os.path.join(out, "calib_verdict.json")))
        assert verdict["raw_ok"] is True and verdict["pass"] is True

        # Pre-fixed rig (0 verts changed and sub-pixel claims) must be rejected by --require-raw.
        prefixed_spec = {
            "check": {
                "beast_a": {"poses": 0.3, "clips": 0.2, "score": 94.0},
                "raw_01": {"poses": 3.2, "clips": 2.8, "score": 71.0},
                "raw_01_fixed": {"poses": 3.2, "clips": 2.8, "score": 71.0},
            },
            "fix": {"raw_01": {"verts_changed": 0, "mean_l1_change": 0.0, "improved": False}},
            "claims": {
                "raw_01": [
                    {"category": "claim", "expected": "tie", "rmse": 0.42, "frame": "walk@0.33s"},
                    {"category": "control", "expected": "cant_tell", "rmse": 0.0, "frame": "walk@0.33s"},
                    {"category": "planted", "expected": "A", "rmse": 20.6, "frame": "trot@0.00s"},
                ]
            },
        }
        make_stub_weights(stub, prefixed_spec)
        out_bad = os.path.join(td, "out_prefixed")
        res_bad = run_calib("--weights", stub, "--artists-dir", art, "--generated-dir", gen, "--out", out_bad, "--require-raw")
        assert res_bad.returncode != 0, "pre-fixed creature must fail --require-raw"
        assert "pre-fixed" in res_bad.stderr, res_bad.stderr
    print("PASS  raw subdir discovery, --gait forwarding, and --require-raw enforcement verified")


if __name__ == "__main__":
    test_raw_subdir_and_require_raw()
