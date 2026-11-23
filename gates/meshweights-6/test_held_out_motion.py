#!/usr/bin/env python3
"""Tests for meshweights-6: kind-specific motion routing in bench/blind_bench.py."""

import hashlib
import json
import os
import stat
import subprocess
import sys
import tempfile

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SCRIPT = os.path.join(REPO, "bench", "blind_bench.py")


def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def make_recording_weights(path: str, log_path: str) -> None:
    code = f"""#!/usr/bin/env python3
import json, os, sys
with open({log_path!r}, "a") as lf:
    lf.write(json.dumps(sys.argv[1:]) + "\\n")
out = sys.argv[sys.argv.index("--out") + 1]
os.makedirs(out, exist_ok=True)
with open(os.path.join(out, "pairs.json"), "w") as f:
    json.dump({{"pairs": []}}, f)
key = os.path.join(os.path.dirname(out), os.path.basename(out) + ".key.json")
with open(key, "w") as f:
    json.dump({{"section": "game clips", "tears_pct": {{"A": 4.0, "B": 2.0}}, "pairs": []}}, f)
sys.exit(0)
"""
    with open(path, "w") as f:
        f.write(code)
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)


def test_kind_specific_motion():
    with tempfile.TemporaryDirectory() as td:
        rigs_dir = os.path.join(td, "rigs")
        os.makedirs(rigs_dir)
        entries = []
        files = {}
        for rid, kind in [
            ("human_01", "human"),
            ("human_02", "human"),
            ("creature_01", "creature"),
            ("creature_02", "creature"),
            ("creature_03", "creature"),
        ]:
            data = f"heldout-{rid}".encode()
            p = os.path.join(rigs_dir, f"{rid}.glb")
            with open(p, "wb") as f:
                f.write(data)
            entries.append({"id": rid, "kind": kind, "sha256": sha256_bytes(data)})
            files[rid] = p
        mf = os.path.join(td, "held_out.json")
        with open(mf, "w") as f:
            json.dump({"version": 1, "frozen_at": "2026-11-19", "rigs": entries}, f)

        hero = os.path.join(td, "hero.glb")
        with open(hero, "wb") as f:
            f.write(b"hero-clips")
        log = os.path.join(td, "calls.jsonl")
        stub = os.path.join(td, "weights_stub.py")
        make_recording_weights(stub, log)

        runs = [f"{k}={files[k]},{files[k]}" for k in files]
        out_dir = os.path.join(td, "out")
        r = subprocess.run(
            [
                sys.executable, SCRIPT,
                "--weights", stub,
                "--out", out_dir,
                "--held-out-manifest", mf,
                "--ranking-claim",
                "--human-motion", hero,
                *runs,
            ],
            capture_output=True,
            text=True,
        )
        assert r.returncode == 0, f"failed: {r.stdout} {r.stderr}"
        with open(log) as f:
            calls = [json.loads(line) for line in f]
        assert len(calls) == 5, f"expected 5 calls, got {len(calls)}"
        by_name = {os.path.basename(c[c.index("--out") + 1]): c for c in calls}
        for hid in ("human_01", "human_02"):
            assert "--motion" in by_name[hid], f"{hid} missing --motion: {by_name[hid]}"
            assert by_name[hid][by_name[hid].index("--motion") + 1] == hero
        for cid in ("creature_01", "creature_02", "creature_03"):
            assert "--motion" not in by_name[cid], f"{cid} should not get human --motion: {by_name[cid]}"

        # Case 2: --creature-motion alone routes to creatures and leaves humans without --motion.
        c_gait = os.path.join(td, "creature_gait.glb")
        with open(c_gait, "wb") as f:
            f.write(b"creature-clips")
        log2 = os.path.join(td, "calls2.jsonl")
        stub2 = os.path.join(td, "weights_stub2.py")
        make_recording_weights(stub2, log2)
        r2 = subprocess.run(
            [
                sys.executable, SCRIPT,
                "--weights", stub2,
                "--out", os.path.join(td, "out2"),
                "--held-out-manifest", mf,
                "--ranking-claim",
                "--creature-motion", c_gait,
                *runs,
            ],
            capture_output=True,
            text=True,
        )
        assert r2.returncode == 0, f"creature-motion failed: {r2.stdout} {r2.stderr}"
        with open(log2) as f:
            calls2 = [json.loads(line) for line in f]
        by_name2 = {os.path.basename(c[c.index("--out") + 1]): c for c in calls2}
        for hid in ("human_01", "human_02"):
            assert "--motion" not in by_name2[hid], f"{hid} should not get creature --motion: {by_name2[hid]}"
        for cid in ("creature_01", "creature_02", "creature_03"):
            assert "--motion" in by_name2[cid], f"{cid} missing --motion: {by_name2[cid]}"
            assert by_name2[cid][by_name2[cid].index("--motion") + 1] == c_gait

        # Case 3: both --human-motion and --creature-motion route each kind to its own clip file.
        log3 = os.path.join(td, "calls3.jsonl")
        stub3 = os.path.join(td, "weights_stub3.py")
        make_recording_weights(stub3, log3)
        r3 = subprocess.run(
            [
                sys.executable, SCRIPT,
                "--weights", stub3,
                "--out", os.path.join(td, "out3"),
                "--held-out-manifest", mf,
                "--ranking-claim",
                "--human-motion", hero,
                "--creature-motion", c_gait,
                *runs,
            ],
            capture_output=True,
            text=True,
        )
        assert r3.returncode == 0, f"combined motion failed: {r3.stdout} {r3.stderr}"
        with open(log3) as f:
            calls3 = [json.loads(line) for line in f]
        by_name3 = {os.path.basename(c[c.index("--out") + 1]): c for c in calls3}
        for hid in ("human_01", "human_02"):
            assert by_name3[hid][by_name3[hid].index("--motion") + 1] == hero
        for cid in ("creature_01", "creature_02", "creature_03"):
            assert by_name3[cid][by_name3[cid].index("--motion") + 1] == c_gait

        # Case 4: fallback --motion applies to both kinds when kind-specific flags are omitted.
        log4 = os.path.join(td, "calls4.jsonl")
        stub4 = os.path.join(td, "weights_stub4.py")
        make_recording_weights(stub4, log4)
        r4 = subprocess.run(
            [
                sys.executable, SCRIPT,
                "--weights", stub4,
                "--out", os.path.join(td, "out4"),
                "--held-out-manifest", mf,
                "--ranking-claim",
                "--motion", hero,
                *runs,
            ],
            capture_output=True,
            text=True,
        )
        assert r4.returncode == 0, f"fallback motion failed: {r4.stdout} {r4.stderr}"
        with open(log4) as f:
            calls4 = [json.loads(line) for line in f]
        by_name4 = {os.path.basename(c[c.index("--out") + 1]): c for c in calls4}
        for rid in files:
            assert by_name4[rid][by_name4[rid].index("--motion") + 1] == hero
    print("PASS  --human-motion, --creature-motion, combined, and fallback --motion routing verified")


if __name__ == "__main__":
    test_kind_specific_motion()
