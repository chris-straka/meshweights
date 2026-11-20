#!/usr/bin/env python3
"""Tests for meshweights-1: bench/creature_calib.py calibration and refusal gates."""

import json
import os
import stat
import subprocess
import sys
import tempfile

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SCRIPT = os.path.join(REPO, "bench", "creature_calib.py")


def run_cli(*args):
    return subprocess.run(
        [sys.executable, SCRIPT, *args],
        capture_output=True,
        text=True,
    )


def make_stub_weights(path: str, spec: dict) -> None:
    """Writes a deterministic stand-in `weights` CLI driven by `spec` JSON."""
    spec_json = json.dumps(spec)
    code = f"""#!/usr/bin/env python3
import json, os, shutil, sys

SPEC = json.loads({spec_json!r})
cmd = sys.argv[1]

def stem(p):
    return os.path.splitext(os.path.basename(p))[0]

if cmd == "check":
    inp = sys.argv[2]
    key = stem(inp)
    entry = SPEC.get("check", {{}}).get(key)
    if entry == "CRASH":
        sys.stderr.write("simulated weights check crash\\n")
        sys.exit(2)
    if entry is None:
        entry = {{"poses": 0.5, "clips": 0.4, "score": 90.0}}
    headline = []
    if entry.get("clips") is not None:
        headline.append({{
            "name": "game clips",
            "frames": 12,
            "clips": ["walk", "trot"],
            "source": "own",
            "tears_pct": entry["clips"],
            "tears_verts_pct": entry["clips"],
            "through_cm2": 0.0,
            "findings": [],
        }})
    headline.append({{
        "name": "poses",
        "frames": 10,
        "clips": [],
        "source": "range of motion",
        "tears_pct": entry["poses"],
        "tears_verts_pct": entry["poses"],
        "through_cm2": 0.0,
        "findings": [],
    }})
    doc = {{
        "file": os.path.basename(inp),
        "class": "quadruped",
        "score": entry.get("score", 80.0),
        "pass": entry["poses"] < 1.0,
        "headline": headline,
        "findings": [],
    }}
    print(json.dumps(doc))
    sys.exit(0 if doc["pass"] else 1)

if cmd == "fix":
    inp = sys.argv[2]
    out = sys.argv[sys.argv.index("--out") + 1]
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    shutil.copyfile(inp, out)
    key = stem(inp)
    entry = SPEC.get("fix", {{}}).get(key, {{"before_poses": 5.0, "after_poses": 2.0, "before_clips": 4.0, "after_clips": 1.5}})
    rep = {{
        "before": {{"score": 50.0, "pass": False, "fails": 2}},
        "after": {{"score": 75.0, "pass": False, "fails": 1}},
        "tears_before": {{"poses": entry["before_poses"], "game clips": entry["before_clips"]}},
        "tears_after": {{"poses": entry["after_poses"], "game clips": entry["after_clips"]}},
    }}
    with open(os.path.splitext(out)[0] + ".report.json", "w") as f:
        json.dump(rep, f)
    sys.exit(0)

if cmd == "blind":
    out = sys.argv[sys.argv.index("--out") + 1]
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "pairs.json"), "w") as f:
        json.dump({{"pairs": [{{"pair": 1}}, {{"pair": 2}}, {{"pair": 3}}]}}, f)
    key_path = os.path.join(os.path.dirname(out), os.path.basename(out) + ".key.json")
    with open(key_path, "w") as f:
        json.dump({{"section": "game clips", "tears_pct": {{"A": 4.0, "B": 1.5}}, "pairs": []}}, f)
    sys.exit(0)

if cmd == "blind-score":
    bdir = sys.argv[2]
    name = os.path.basename(os.path.normpath(bdir))
    v = SPEC.get("verdict", {{}}).get(name, {{
        "pass": True,
        "control_ok": True,
        "planted_ok": True,
        "planted_tears_ok": True,
        "claims_judged": 2,
        "judge_agrees_tears": 2,
        "score_agrees_judge": 1,
        "score_retired_pairs": [1],
    }})
    with open(os.path.join(bdir, "verdict.json"), "w") as f:
        json.dump(v, f)
    sys.exit(0 if v.get("pass") else 1)

sys.exit(2)
"""
    with open(path, "w") as f:
        f.write(code)
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)


def make_dirs(td: str):
    art_dir = os.path.join(td, "artists")
    gen_dir = os.path.join(td, "generated")
    os.makedirs(art_dir)
    os.makedirs(gen_dir)
    for i in (1, 2, 3):
        open(os.path.join(art_dir, f"artist_{i:02d}.glb"), "wb").write(f"art-{i}".encode())
    for i in (1, 2, 3):
        open(os.path.join(gen_dir, f"gen_{i:02d}.glb"), "wb").write(f"gen-{i}".encode())
    return art_dir, gen_dir


def write_answers(out_dir: str, names):
    for n in names:
        p = os.path.join(out_dir, "blind", n, "answers.json")
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w") as f:
            json.dump({"answers": {"1": "left", "2": "cant_tell", "3": "right"}}, f)


def base_spec():
    return {
        "check": {
            "artist_01": {"poses": 0.2, "clips": 0.1, "score": 92.0},
            "artist_02": {"poses": 0.5, "clips": 0.4, "score": 88.0},
            "artist_03": {"poses": 0.8, "clips": 0.6, "score": 85.0},
            "gen_01": {"poses": 6.0, "clips": 3.5, "score": 52.0},
            "gen_02": {"poses": 4.5, "clips": 2.8, "score": 60.0},
            "gen_03": {"poses": 3.0, "clips": 2.0, "score": 68.0},
            "gen_01_fixed": {"poses": 2.5, "clips": 1.8, "score": 72.0},
            "gen_02_fixed": {"poses": 2.2, "clips": 1.6, "score": 74.0},
            "gen_03_fixed": {"poses": 1.5, "clips": 1.2, "score": 78.0},
        },
        "fix": {
            "gen_01": {"before_poses": 6.0, "after_poses": 2.5, "before_clips": 3.5, "after_clips": 1.8},
            "gen_02": {"before_poses": 4.5, "after_poses": 2.2, "before_clips": 2.8, "after_clips": 1.6},
            "gen_03": {"before_poses": 3.0, "after_poses": 1.5, "before_clips": 2.0, "after_clips": 1.2},
        },
        "verdict": {
            "gen_01": {"pass": True, "control_ok": True, "planted_ok": True, "planted_tears_ok": True, "claims_judged": 2, "judge_agrees_tears": 2},
            "gen_02": {"pass": True, "control_ok": True, "planted_ok": True, "planted_tears_ok": True, "claims_judged": 2, "judge_agrees_tears": 2},
            "gen_03": {"pass": True, "control_ok": True, "planted_ok": True, "planted_tears_ok": True, "claims_judged": 1, "judge_agrees_tears": 0},
        },
    }


def test_clean_calibration_passes():
    # 4/5 = 80.0% blind agreement, artist median (0.5/0.4) < generated median (4.5/2.8), 0 worse -> PASS.
    with tempfile.TemporaryDirectory() as td:
        art, gen = make_dirs(td)
        stub = os.path.join(td, "weights_stub.py")
        make_stub_weights(stub, base_spec())
        out = os.path.join(td, "out")
        r = run_cli("--weights", stub, "--artists-dir", art, "--generated-dir", gen, "--out", out)
        assert r.returncode == 0, f"measure failed: {r.stdout} {r.stderr}"
        assert os.path.isfile(os.path.join(out, "calib.json"))
        assert os.path.isfile(os.path.join(out, "calib.md"))
        calib = json.load(open(os.path.join(out, "calib.json")))
        assert calib["artist_median_poses"] < calib["generated_before_median_poses"]
        assert calib["artist_median_clips"] < calib["generated_before_median_clips"]

        write_answers(out, ("gen_01", "gen_02", "gen_03"))
        rs = run_cli("--weights", stub, "--score", out)
        assert rs.returncode == 0, f"score failed: {rs.stdout} {rs.stderr}"
        v = json.load(open(os.path.join(out, "calib_verdict.json")))
        assert v["pass"] is True
        assert abs(v["agreement_rate"] - 0.8) < 1e-6
    print("PASS  clean calibration (artist < generated medians, 0 worse, 80% blind agreement, gates ok) passes")


def test_inverted_medians_fails():
    with tempfile.TemporaryDirectory() as td:
        art, gen = make_dirs(td)
        spec = base_spec()
        spec["check"]["artist_01"] = {"poses": 7.0, "clips": 5.0, "score": 40.0}
        spec["check"]["artist_02"] = {"poses": 6.5, "clips": 4.5, "score": 42.0}
        spec["check"]["artist_03"] = {"poses": 6.0, "clips": 4.0, "score": 45.0}
        stub = os.path.join(td, "weights_stub.py")
        make_stub_weights(stub, spec)
        out = os.path.join(td, "out")
        r = run_cli("--weights", stub, "--artists-dir", art, "--generated-dir", gen, "--out", out)
        assert r.returncode != 0, "artist median >= generated median must fail"
        write_answers(out, ("gen_01", "gen_02", "gen_03"))
        rs = run_cli("--weights", stub, "--score", out)
        assert rs.returncode != 0, "score must fail when artist median >= generated median"
    print("PASS  inverted artist vs generated medians rejected")


def test_worse_after_fix_fails():
    with tempfile.TemporaryDirectory() as td:
        art, gen = make_dirs(td)
        spec = base_spec()
        spec["check"]["gen_02_fixed"] = {"poses": 5.5, "clips": 3.5, "score": 50.0}
        spec["fix"]["gen_02"]["after_poses"] = 5.5
        spec["fix"]["gen_02"]["after_clips"] = 3.5
        stub = os.path.join(td, "weights_stub.py")
        make_stub_weights(stub, spec)
        out = os.path.join(td, "out")
        r = run_cli("--weights", stub, "--artists-dir", art, "--generated-dir", gen, "--out", out)
        assert r.returncode != 0, "worse tears after fix must fail"
    print("PASS  worse tears after fix rejected")


def test_low_agreement_fails():
    # 3/5 = 60% < 80% -> must fail.
    with tempfile.TemporaryDirectory() as td:
        art, gen = make_dirs(td)
        spec = base_spec()
        spec["verdict"]["gen_02"]["judge_agrees_tears"] = 1
        stub = os.path.join(td, "weights_stub.py")
        make_stub_weights(stub, spec)
        out = os.path.join(td, "out")
        assert run_cli("--weights", stub, "--artists-dir", art, "--generated-dir", gen, "--out", out).returncode == 0
        write_answers(out, ("gen_01", "gen_02", "gen_03"))
        rs = run_cli("--weights", stub, "--score", out)
        assert rs.returncode != 0, "agreement < 80% must fail"
    print("PASS  blind agreement < 80% rejected")


def test_failed_control_or_planted_or_zero_claims_fails():
    for label, patch in [
        ("control_ok=False", {"control_ok": False, "pass": False}),
        ("planted_ok=False", {"planted_ok": False, "pass": False}),
        ("planted_tears_ok=False", {"planted_tears_ok": False, "pass": False}),
        ("zero claims judged", None),
    ]:
        with tempfile.TemporaryDirectory() as td:
            art, gen = make_dirs(td)
            spec = base_spec()
            if patch is None:
                for k in spec["verdict"]:
                    spec["verdict"][k]["claims_judged"] = 0
                    spec["verdict"][k]["judge_agrees_tears"] = 0
            else:
                spec["verdict"]["gen_01"].update(patch)
            stub = os.path.join(td, "weights_stub.py")
            make_stub_weights(stub, spec)
            out = os.path.join(td, "out")
            assert run_cli("--weights", stub, "--artists-dir", art, "--generated-dir", gen, "--out", out).returncode == 0
            write_answers(out, ("gen_01", "gen_02", "gen_03"))
            rs = run_cli("--weights", stub, "--score", out)
            assert rs.returncode != 0, f"{label} must fail"
    print("PASS  failed control/planted and zero-claims runs rejected")


def test_crashed_check_fails_loudly():
    with tempfile.TemporaryDirectory() as td:
        art, gen = make_dirs(td)
        spec = base_spec()
        spec["check"]["artist_02"] = "CRASH"
        stub = os.path.join(td, "weights_stub.py")
        make_stub_weights(stub, spec)
        out = os.path.join(td, "out")
        r = run_cli("--weights", stub, "--artists-dir", art, "--generated-dir", gen, "--out", out)
        assert r.returncode != 0, "crashed weights check must fail loudly, not score 0"
    print("PASS  crashed weights check fails loudly")


if __name__ == "__main__":
    test_clean_calibration_passes()
    test_inverted_medians_fails()
    test_worse_after_fix_fails()
    test_low_agreement_fails()
    test_failed_control_or_planted_or_zero_claims_fails()
    test_crashed_check_fails_loudly()
    print("ALL TESTS PASSED")
