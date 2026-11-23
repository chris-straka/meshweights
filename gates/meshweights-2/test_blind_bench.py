#!/usr/bin/env python3
"""Tests for meshweights-2: frozen held-out manifest and blind_bench.py refusal."""

import hashlib
import json
import os
import re
import stat
import subprocess
import sys
import tempfile

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SCRIPT = os.path.join(REPO, "bench", "blind_bench.py")
MANIFEST = os.path.join(REPO, "bench", "held_out.json")


def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def run_cli(*args):
    return subprocess.run(
        [sys.executable, SCRIPT, *args],
        capture_output=True,
        text=True,
    )


def make_stub_weights(path: str) -> None:
    code = """#!/usr/bin/env python3
import json, os, sys
cmd = sys.argv[1]
if cmd == "blind":
    out = sys.argv[sys.argv.index("--out") + 1]
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "pairs.json"), "w") as f:
        json.dump({"pairs": []}, f)
    key = os.path.join(os.path.dirname(out), os.path.basename(out) + ".key.json")
    with open(key, "w") as f:
        json.dump({"section": "game clips", "tears_pct": {"A": 5.0, "B": 2.0}, "pairs": []}, f)
    sys.exit(0)
if cmd == "blind-score":
    sys.exit(0)
sys.exit(2)
"""
    with open(path, "w") as f:
        f.write(code)
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)


def make_fake_heldout(td: str):
    """Creates 5 synthetic files (2 humans, 3 creatures) and a matching temp manifest."""
    rigs_dir = os.path.join(td, "rigs")
    os.makedirs(rigs_dir, exist_ok=True)
    entries = []
    specs = [
        ("human_01", "human"),
        ("human_02", "human"),
        ("creature_01", "creature"),
        ("creature_02", "creature"),
        ("creature_03", "creature"),
    ]
    files = {}
    for rid, kind in specs:
        data = f"synthetic-heldout-{rid}-{kind}".encode()
        p = os.path.join(rigs_dir, f"{rid}.glb")
        with open(p, "wb") as f:
            f.write(data)
        entries.append({"id": rid, "kind": kind, "sha256": sha256_bytes(data)})
        files[rid] = p
    mf = os.path.join(td, "held_out.json")
    with open(mf, "w") as f:
        json.dump({"version": 1, "frozen_at": "2026-11-19", "rigs": entries}, f, indent=2)
    return mf, rigs_dir, files


def test_real_manifest_format():
    with open(MANIFEST) as f:
        doc = json.load(f)
    rigs = doc.get("rigs", [])
    humans = [r for r in rigs if r.get("kind") == "human"]
    creatures = [r for r in rigs if r.get("kind") == "creature"]
    assert len(humans) >= 2, f"need >=2 humans, got {len(humans)}"
    assert len(creatures) >= 3, f"need >=3 creatures, got {len(creatures)}"
    seen_sha = set()
    for r in rigs:
        assert re.fullmatch(r"[0-9a-f]{64}", r["sha256"]), f"bad sha256: {r}"
        assert r["sha256"] not in seen_sha, f"duplicate sha256: {r}"
        seen_sha.add(r["sha256"])
        assert "/" not in r["id"] and "\\" not in r["id"], f"path in id: {r}"
    print("PASS  real bench/held_out.json manifest schema")


def test_verify_manifest():
    with tempfile.TemporaryDirectory() as td:
        mf, rigs_dir, files = make_fake_heldout(td)
        r = run_cli("--verify-manifest", rigs_dir, "--held-out-manifest", mf)
        assert r.returncode == 0, f"expected pass on valid held-out dir: {r.stdout} {r.stderr}"

        # Tamper 1 byte in creature_02 -> must refuse.
        with open(files["creature_02"], "ab") as f:
            f.write(b"X")
        r = run_cli("--verify-manifest", rigs_dir, "--held-out-manifest", mf)
        assert r.returncode != 0, "expected refusal on tampered held-out file"
        assert "creature_02" in (r.stdout + r.stderr) or "sha256" in (r.stdout + r.stderr).lower()

        # Remove human_01 -> must refuse.
        os.remove(files["human_01"])
        r = run_cli("--verify-manifest", rigs_dir, "--held-out-manifest", mf)
        assert r.returncode != 0, "expected refusal on missing held-out file"
    print("PASS  verify-manifest passes clean dir and refuses tampered/missing files")


def test_ranking_claim_pair_generation_refusals():
    with tempfile.TemporaryDirectory() as td:
        mf, _, files = make_fake_heldout(td)
        stub = os.path.join(td, "weights_stub.py")
        make_stub_weights(stub)

        # Dev-only file (not in manifest) with --ranking-claim -> must refuse.
        dev = os.path.join(td, "dev.glb")
        with open(dev, "wb") as f:
            f.write(b"not-a-heldout-rig")
        r = run_cli("--weights", stub, "--out", os.path.join(td, "out_dev"),
                    "--held-out-manifest", mf, "--ranking-claim", f"dev={dev},{dev}")
        assert r.returncode != 0, "ranking claim without held-out rigs must fail"
        assert "held-out" in (r.stdout + r.stderr).lower(), f"message should name held-out: {r.stdout} {r.stderr}"

        # Partial held-out (4 of 5) -> must refuse.
        four = [f"{k}={files[k]},{files[k]}" for k in ("human_01", "human_02", "creature_01", "creature_02")]
        r = run_cli("--weights", stub, "--out", os.path.join(td, "out_four"),
                    "--held-out-manifest", mf, "--ranking-claim", *four)
        assert r.returncode != 0, "ranking claim missing 1 held-out rig must fail"

        # All 5 matching frozen sha256s -> must succeed and write run_manifest.json.
        five = [f"{k}={files[k]},{files[k]}" for k in ("human_01", "human_02", "creature_01", "creature_02", "creature_03")]
        out_ok = os.path.join(td, "out_ok")
        r = run_cli("--weights", stub, "--out", out_ok,
                    "--held-out-manifest", mf, "--ranking-claim", *five)
        assert r.returncode == 0, f"ranking claim with all 5 held-out rigs failed: {r.stdout} {r.stderr}"
        rm = os.path.join(out_ok, "run_manifest.json")
        assert os.path.isfile(rm), "run_manifest.json must be written"
        with open(rm) as f:
            doc = json.load(f)
        assert len(doc.get("runs", [])) == 5
    print("PASS  ranking-claim refuses missing/partial held-out and accepts complete frozen set")


def write_batch(batch_dir: str, mf: str, files: dict, outcomes: dict):
    """Writes run_manifest.json, per-run .key.json, and verdict.json."""
    os.makedirs(batch_dir, exist_ok=True)
    runs = []
    for rid, p in files.items():
        ta, tb, ctrl_ok, plant_ok, plant_tears_ok = outcomes[rid]
        rdir = os.path.join(batch_dir, rid)
        os.makedirs(rdir, exist_ok=True)
        with open(os.path.join(batch_dir, f"{rid}.key.json"), "w") as f:
            json.dump({
                "rig_a": f"{rid}.glb",
                "rig_b": f"{rid}.fixed.glb",
                "section": "game clips",
                "tears_pct": {"A": ta, "B": tb},
                "pairs": [],
            }, f)
        with open(os.path.join(rdir, "verdict.json"), "w") as f:
            json.dump({
                "pass": ctrl_ok and plant_ok and plant_tears_ok,
                "control_ok": ctrl_ok,
                "planted_ok": plant_ok,
                "planted_tears_ok": plant_tears_ok,
            }, f)
        with open(p, "rb") as f:
            sha_a = sha256_bytes(f.read())
        runs.append({
            "name": rid,
            "sha256_a": sha_a,
            "sha256_b": sha256_bytes(b"fixed-" + rid.encode()),
        })
    with open(os.path.join(batch_dir, "run_manifest.json"), "w") as f:
        json.dump({"runs": runs}, f, indent=2)


def test_score_batch_ranking_claim():
    with tempfile.TemporaryDirectory() as td:
        mf, _, files = make_fake_heldout(td)

        # Valid batch: 0 worse, 3/5 improved, all controls & planted ok -> PASS.
        good = {
            "human_01": (2.28, 2.28, True, True, True),
            "human_02": (4.15, 4.15, True, True, True),
            "creature_01": (8.75, 3.09, True, True, True),
            "creature_02": (5.22, 4.89, True, True, True),
            "creature_03": (0.85, 0.46, True, True, True),
        }
        b_ok = os.path.join(td, "batch_ok")
        write_batch(b_ok, mf, files, good)
        r = run_cli("--score-batch", b_ok, "--ranking-claim", "--held-out-manifest", mf)
        assert r.returncode == 0, f"valid held-out batch should pass: {r.stdout} {r.stderr}"

        # Worse tears on 1 rig -> MUST REFUSE.
        worse = dict(good, human_01=(2.28, 2.50, True, True, True))
        b_worse = os.path.join(td, "batch_worse")
        write_batch(b_worse, mf, files, worse)
        r = run_cli("--score-batch", b_worse, "--ranking-claim", "--held-out-manifest", mf)
        assert r.returncode != 0, "worse tears on a held-out rig must fail ranking claim"

        # Failed control on 1 rig -> MUST REFUSE.
        bad_ctrl = dict(good, creature_02=(5.22, 4.89, False, True, True))
        b_ctrl = os.path.join(td, "batch_ctrl")
        write_batch(b_ctrl, mf, files, bad_ctrl)
        r = run_cli("--score-batch", b_ctrl, "--ranking-claim", "--held-out-manifest", mf)
        assert r.returncode != 0, "failed control on held-out rig must fail ranking claim"

        # Fewer than majority improved (only 2/5) -> MUST REFUSE.
        few = dict(good, creature_02=(5.22, 5.22, True, True, True))
        b_few = os.path.join(td, "batch_few")
        write_batch(b_few, mf, files, few)
        r = run_cli("--score-batch", b_few, "--ranking-claim", "--held-out-manifest", mf)
        assert r.returncode != 0, "ranking claim must refuse when tears did not improve on most rigs"

        # Missing held-out rig in run_manifest.json -> MUST REFUSE.
        b_miss = os.path.join(td, "batch_miss")
        write_batch(b_miss, mf, {k: v for k, v in files.items() if k != "creature_03"}, good)
        r = run_cli("--score-batch", b_miss, "--ranking-claim", "--held-out-manifest", mf)
        assert r.returncode != 0, "ranking claim must refuse when a held-out rig is missing"
    print("PASS  score-batch enforces full held-out coverage, never-worse, majority-improved, and control/planted gates")


if __name__ == "__main__":
    test_real_manifest_format()
    test_verify_manifest()
    test_ranking_claim_pair_generation_refusals()
    test_score_batch_ranking_claim()
    print("ALL TESTS PASSED")
