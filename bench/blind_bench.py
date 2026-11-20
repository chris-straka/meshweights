#!/usr/bin/env python3
"""Same-frame blind pairs for rig claims (plain Python 3, stdlib only).

    python3 bench/blind_bench.py --verify-manifest DIR [--held-out-manifest PATH]
    python3 bench/blind_bench.py --weights rust/target/release/weights \\
        --out DIR NAME=a.glb,b.glb [NAME=a.glb,b.glb ...] [--seed N] [--top N] \\
        [--ranking-claim] [--held-out-manifest PATH]
    python3 bench/blind_bench.py --score DIR/NAME --answers answers.json
    python3 bench/blind_bench.py --score-batch DIR [--ranking-claim] \\
        [--held-out-manifest PATH]

The pair-generation form writes one blind run per NAME into DIR/NAME (pairs
for a person to judge, key beside it as NAME.key.json — keep the key away
from the judge) and records input/output SHA-256 digests in
DIR/run_manifest.json. With --ranking-claim, pair generation refuses to run
unless every frozen held-out rig in --held-out-manifest appears as input A,
and --score-batch refuses unless all frozen held-out rigs are present, pass
their control/planted blind gates, never worsen on tears, and improve on
tears on a strict majority of held-out rigs.
See docs/blind.md.
"""

import argparse
import hashlib
import json
import math
import os
import re
import subprocess
import sys

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
DEFAULT_MANIFEST = os.path.join(SCRIPT_DIR, "held_out.json")
SHA256_RE = re.compile(r"^[0-9a-fA-F]{64}$")
MIN_HUMANS = 2
MIN_CREATURES = 3
TEARS_EPS = 1e-9


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            chunk = f.read(65536)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


def load_held_out_manifest(path: str) -> list:
    if not os.path.isfile(path):
        raise ValueError(f"held-out manifest not found: {path}")
    try:
        with open(path, "r", encoding="utf-8") as f:
            doc = json.load(f)
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"failed to read held-out manifest {path}: {exc}") from exc

    if not isinstance(doc, dict) or not isinstance(doc.get("rigs"), list):
        raise ValueError(f"invalid held-out manifest {path}: expected object with 'rigs' list")

    rigs = []
    seen_ids = set()
    seen_shas = set()
    humans = 0
    creatures = 0

    for idx, entry in enumerate(doc["rigs"]):
        if not isinstance(entry, dict):
            raise ValueError(f"invalid held-out manifest {path}: rigs[{idx}] must be an object")
        rid = entry.get("id")
        kind = entry.get("kind")
        sha = entry.get("sha256")
        if not isinstance(rid, str) or not rid or "/" in rid or "\\" in rid or rid in (".", ".."):
            raise ValueError(f"invalid held-out manifest {path}: rigs[{idx}] has invalid id {rid!r}")
        if not isinstance(kind, str) or not kind:
            raise ValueError(f"invalid held-out manifest {path}: rigs[{idx}] ({rid}) has invalid kind {kind!r}")
        if not isinstance(sha, str) or not SHA256_RE.match(sha):
            raise ValueError(
                f"invalid held-out manifest {path}: rigs[{idx}] ({rid}) has invalid sha256 {sha!r}"
            )
        sha_norm = sha.lower()
        if rid in seen_ids:
            raise ValueError(f"invalid held-out manifest {path}: duplicate id {rid!r}")
        if sha_norm in seen_shas:
            raise ValueError(f"invalid held-out manifest {path}: duplicate sha256 {sha_norm}")
        seen_ids.add(rid)
        seen_shas.add(sha_norm)
        if kind == "human":
            humans += 1
        elif kind == "creature":
            creatures += 1
        rigs.append({"id": rid, "kind": kind, "sha256": sha_norm})

    if humans < MIN_HUMANS or creatures < MIN_CREATURES:
        raise ValueError(
            f"invalid held-out manifest {path}: need at least {MIN_HUMANS} human and "
            f"{MIN_CREATURES} creature entries, got {humans} human and {creatures} creature"
        )
    return rigs


def cmd_verify_manifest(verify_dir: str, manifest_path: str) -> int:
    try:
        rigs = load_held_out_manifest(manifest_path)
    except ValueError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2

    if not os.path.isdir(verify_dir):
        print(f"ERROR: held-out directory does not exist: {verify_dir}", file=sys.stderr)
        return 2

    # Lazily hash other .glb files in verify_dir only if <id>.glb is absent.
    scanned_glbs = None

    def get_scanned_glbs() -> dict:
        nonlocal scanned_glbs
        if scanned_glbs is None:
            scanned_glbs = {}
            for fname in sorted(os.listdir(verify_dir)):
                if not fname.lower().endswith(".glb"):
                    continue
                fpath = os.path.join(verify_dir, fname)
                if not os.path.isfile(fpath):
                    continue
                digest = sha256_file(fpath)
                scanned_glbs.setdefault(digest, []).append(fname)
        return scanned_glbs

    errors = []
    matched = []
    used_files = set()

    for entry in rigs:
        rid = entry["id"]
        expected = entry["sha256"]
        preferred_name = f"{rid}.glb"
        preferred_path = os.path.join(verify_dir, preferred_name)

        if os.path.isfile(preferred_path):
            actual = sha256_file(preferred_path)
            if actual == expected:
                matched.append((rid, entry["kind"], preferred_name, expected))
                used_files.add(preferred_name)
            else:
                errors.append(
                    f"held-out rig {rid!r} mismatch in {preferred_name}: "
                    f"expected sha256 {expected}, got {actual}"
                )
        else:
            candidates = [f for f in get_scanned_glbs().get(expected, []) if f not in used_files]
            if candidates:
                chosen = candidates[0]
                matched.append((rid, entry["kind"], chosen, expected))
                used_files.add(chosen)
            else:
                errors.append(
                    f"held-out rig {rid!r} missing: expected {preferred_name} "
                    f"(or any .glb in directory) with sha256 {expected}"
                )

    if errors:
        print(f"FAILED held-out manifest verification ({len(errors)} error(s)):", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    for rid, kind, fname, digest in matched:
        print(f"OK  {rid:<14} ({kind:<8}) {fname}  sha256={digest[:12]}...")
    print(f"PASS held-out manifest verified: {len(matched)}/{len(rigs)} rigs matched")
    return 0


def _is_finite_number(val) -> bool:
    return isinstance(val, (int, float)) and not isinstance(val, bool) and math.isfinite(val)


def _read_json_file(path: str):
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


def cmd_score_batch(batch_dir: str, manifest_path: str, manifest_explicit: bool, ranking_claim: bool) -> int:
    held_out = []
    by_sha = {}
    if ranking_claim or manifest_explicit or os.path.isfile(manifest_path):
        try:
            held_out = load_held_out_manifest(manifest_path)
            by_sha = {r["sha256"]: r for r in held_out}
        except ValueError as exc:
            print(f"ERROR: {exc}", file=sys.stderr)
            return 2

    manifest_file = os.path.join(batch_dir, "run_manifest.json")
    if not os.path.isfile(manifest_file):
        print(
            f"ERROR: held-out batch missing run_manifest.json in {batch_dir}",
            file=sys.stderr,
        )
        return 2
    try:
        rm_doc = _read_json_file(manifest_file)
    except (OSError, json.JSONDecodeError) as exc:
        print(f"ERROR: failed to read {manifest_file}: {exc}", file=sys.stderr)
        return 2

    runs = rm_doc.get("runs") if isinstance(rm_doc, dict) else None
    if not isinstance(runs, list):
        print(f"ERROR: invalid {manifest_file}: expected object with 'runs' list", file=sys.stderr)
        return 2

    refusal_reasons = []
    batch_errors = []
    rows = []
    runs_by_held_out_sha = {r["sha256"]: [] for r in held_out}

    for idx, run in enumerate(runs):
        if not isinstance(run, dict):
            batch_errors.append(f"run_manifest.json runs[{idx}] is not an object")
            continue
        name = run.get("name")
        sha_a = run.get("sha256_a")
        if not isinstance(name, str) or not name or "/" in name or "\\" in name or name in (".", ".."):
            batch_errors.append(f"run_manifest.json runs[{idx}] has invalid name {name!r}")
            continue
        sha_a_norm = sha_a.lower() if isinstance(sha_a, str) else ""
        ho_entry = by_sha.get(sha_a_norm)

        key_path = os.path.join(batch_dir, f"{name}.key.json")
        verdict_path = os.path.join(batch_dir, name, "verdict.json")

        key_err = None
        tears_a = None
        tears_b = None
        if not os.path.isfile(key_path):
            key_err = f"missing {name}.key.json"
        else:
            try:
                key_doc = _read_json_file(key_path)
                tp = key_doc.get("tears_pct") if isinstance(key_doc, dict) else None
                if not isinstance(tp, dict) or not _is_finite_number(tp.get("A")) or not _is_finite_number(tp.get("B")):
                    key_err = f"invalid tears_pct in {name}.key.json"
                else:
                    tears_a = float(tp["A"])
                    tears_b = float(tp["B"])
            except (OSError, json.JSONDecodeError) as exc:
                key_err = f"unreadable {name}.key.json ({exc})"

        verdict_err = None
        gates_ok = False
        gate_detail = ""
        if not os.path.isfile(verdict_path):
            verdict_err = f"missing {name}/verdict.json"
        else:
            try:
                v_doc = _read_json_file(verdict_path)
                if not isinstance(v_doc, dict):
                    verdict_err = f"invalid {name}/verdict.json"
                else:
                    failed_gates = []
                    for flag in ("control_ok", "planted_ok", "planted_tears_ok"):
                        if v_doc.get(flag) is not True:
                            failed_gates.append(flag)
                    if v_doc.get("pass") is False and not failed_gates:
                        failed_gates.append("pass")
                    if failed_gates:
                        gate_detail = "FAIL(" + ",".join(failed_gates) + ")"
                    else:
                        gates_ok = True
                        gate_detail = "PASS"
            except (OSError, json.JSONDecodeError) as exc:
                verdict_err = f"unreadable {name}/verdict.json ({exc})"

        info = {
            "name": name,
            "held_out": ho_entry,
            "tears_a": tears_a,
            "tears_b": tears_b,
            "key_err": key_err,
            "verdict_err": verdict_err,
            "gates_ok": gates_ok,
            "gate_detail": gate_detail or ("MISSING" if verdict_err else "FAIL"),
        }
        rows.append(info)
        if ho_entry is not None:
            runs_by_held_out_sha[ho_entry["sha256"]].append(info)

        if key_err:
            batch_errors.append(f"run {name!r}: {key_err}")
        if verdict_err:
            batch_errors.append(f"run {name!r}: {verdict_err}")
        elif not gates_ok:
            batch_errors.append(f"run {name!r}: blind validity gate failed ({gate_detail})")

    # Print concise summary table.
    print(f"{'run':<16} {'held-out':<14} {'tears A -> B':<18} {'delta':>8}  {'gates'}")
    print("-" * 70)
    for info in rows:
        ho_label = info["held_out"]["id"] if info["held_out"] else "-"
        if info["tears_a"] is not None and info["tears_b"] is not None:
            tears_str = f"{info['tears_a']:5.2f}% -> {info['tears_b']:5.2f}%"
            delta = info["tears_b"] - info["tears_a"]
            delta_str = f"{delta:+7.2f}%"
        else:
            tears_str = "missing"
            delta_str = "n/a"
        print(f"{info['name']:<16} {ho_label:<14} {tears_str:<18} {delta_str:>8}  {info['gate_detail']}")

    if ranking_claim:
        improved_rigs = 0
        worse_rigs = 0
        gates_passed_rigs = 0

        for entry in held_out:
            rid = entry["id"]
            sha = entry["sha256"]
            matched_runs = runs_by_held_out_sha[sha]
            if not matched_runs:
                refusal_reasons.append(
                    f"missing held-out rig {rid!r} (sha256 {sha}) from run_manifest.json"
                )
                continue

            rig_ok = True
            rig_improved = True
            rig_worse = False
            for info in matched_runs:
                if info["key_err"]:
                    refusal_reasons.append(f"held-out rig {rid!r} (run {info['name']!r}): {info['key_err']}")
                    rig_ok = False
                if info["verdict_err"]:
                    refusal_reasons.append(f"held-out rig {rid!r} (run {info['name']!r}): {info['verdict_err']}")
                    rig_ok = False
                elif not info["gates_ok"]:
                    refusal_reasons.append(
                        f"held-out rig {rid!r} (run {info['name']!r}): failed blind validity gates ({info['gate_detail']})"
                    )
                    rig_ok = False
                if info["tears_a"] is not None and info["tears_b"] is not None:
                    if info["tears_b"] > info["tears_a"] + TEARS_EPS:
                        refusal_reasons.append(
                            f"held-out rig {rid!r} (run {info['name']!r}): tears got worse "
                            f"({info['tears_a']:.2f}% -> {info['tears_b']:.2f}%)"
                        )
                        rig_worse = True
                        rig_improved = False
                    elif not (info["tears_b"] < info["tears_a"] - TEARS_EPS):
                        rig_improved = False
                else:
                    rig_improved = False

            if rig_ok:
                gates_passed_rigs += 1
            if rig_worse:
                worse_rigs += 1
            if rig_ok and not rig_worse and rig_improved:
                improved_rigs += 1

        total_ho = len(held_out)
        if improved_rigs * 2 <= total_ho:
            refusal_reasons.append(
                f"fewer than a strict majority of held-out rigs improved on tears "
                f"({improved_rigs}/{total_ho} improved; need > {total_ho / 2:.1f})"
            )

        if refusal_reasons:
            print(
                f"REFUSED ranking claim: held-out verification failed ({len(refusal_reasons)} issue(s)):",
                file=sys.stderr,
            )
            for reason in refusal_reasons:
                print(f"  - {reason}", file=sys.stderr)
            return 1

        print(
            f"PASS held-out ranking claim: {improved_rigs}/{total_ho} improved on tears, "
            f"{worse_rigs} worse, {gates_passed_rigs}/{total_ho} blind gates ok"
        )
        return 0

    if batch_errors:
        print(f"FAILED batch check ({len(batch_errors)} issue(s)):", file=sys.stderr)
        for err in batch_errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    print(f"PASS batch check: {len(rows)} run(s) valid")
    return 0


def cmd_generate_pairs(a, ap, manifest_explicit: bool) -> int:
    if not a.out or not a.runs:
        ap.error("making pairs needs --out and NAME=a.glb,b.glb runs")

    held_out = []
    by_sha = {}
    if a.ranking_claim or manifest_explicit or os.path.isfile(a.held_out_manifest):
        try:
            held_out = load_held_out_manifest(a.held_out_manifest)
            by_sha = {r["sha256"]: r for r in held_out}
        except ValueError as exc:
            print(f"ERROR: {exc}", file=sys.stderr)
            return 2

    parsed_runs = []
    for idx, spec in enumerate(a.runs):
        name, sep, rest = spec.partition("=")
        left, comma, right = rest.partition(",")
        if not sep or not comma or not name or not left or not right:
            ap.error(f"bad run {spec!r}: want NAME=a.glb,b.glb")
        if not os.path.isfile(left):
            print(f"ERROR: run {name!r} input A does not exist: {left}", file=sys.stderr)
            return 2
        if not os.path.isfile(right):
            print(f"ERROR: run {name!r} input B does not exist: {right}", file=sys.stderr)
            return 2
        sha_a = sha256_file(left)
        sha_b = sha256_file(right)
        ho = by_sha.get(sha_a)
        parsed_runs.append(
            {
                "idx": idx,
                "name": name,
                "left": left,
                "right": right,
                "sha256_a": sha_a,
                "sha256_b": sha_b,
                "held_out_id": ho["id"] if ho else None,
                "held_out_kind": ho["kind"] if ho else None,
            }
        )

    if a.ranking_claim:
        present_shas = {r["sha256_a"] for r in parsed_runs}
        missing = [e for e in held_out if e["sha256"] not in present_shas]
        if missing:
            print(
                f"REFUSED ranking claim: missing {len(missing)}/{len(held_out)} frozen held-out rig(s) as input A:",
                file=sys.stderr,
            )
            for e in missing:
                print(
                    f"  - {e['id']} ({e['kind']}) sha256={e['sha256']}",
                    file=sys.stderr,
                )
            return 1

    w = os.path.abspath(a.weights)
    os.makedirs(a.out, exist_ok=True)
    rc = 0
    for run in parsed_runs:
        name = run["name"]
        idx = run["idx"]
        out = os.path.join(a.out, name)
        seed = str(int(a.seed) + idx) if a.seed.lstrip("-").isdigit() else a.seed
        cmd = [w, "blind", run["left"], run["right"], "--out", out, "--seed", seed, "--top", a.top]
        if a.motion:
            cmd += ["--motion", a.motion]
        print(f"== {name}: {' '.join(cmd)}", flush=True)
        r = subprocess.run(cmd)
        rc = rc or r.returncode

    manifest_out = os.path.join(a.out, "run_manifest.json")
    manifest_payload = {
        "version": 1,
        "ranking_claim": bool(a.ranking_claim),
        "runs": [
            {
                "name": run["name"],
                "sha256_a": run["sha256_a"],
                "sha256_b": run["sha256_b"],
                "held_out_id": run["held_out_id"],
                "held_out_kind": run["held_out_kind"],
            }
            for run in parsed_runs
        ],
    }
    with open(manifest_out, "w", encoding="utf-8") as f:
        json.dump(manifest_payload, f, indent=2)
        f.write("\n")

    if rc == 0:
        print(
            "next: judge each pair_NN.png (left/right/cant_tell) without opening any .key.json, "
            "then run with --score RUN --answers answers.json"
        )
    return rc


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--weights",
        default=os.path.join(os.path.dirname(__file__), "..", "rust", "target", "release", "weights"),
    )
    ap.add_argument("--out")
    ap.add_argument("--seed", default="20261115")
    ap.add_argument("--top", default="6")
    ap.add_argument("--motion")
    ap.add_argument("--score", help="a blind run folder to score instead of making pairs")
    ap.add_argument("--answers", help="judge answers JSON (with --score)")
    ap.add_argument("--key", help="answer key JSON (with --score; default: beside the run)")
    ap.add_argument(
        "--held-out-manifest",
        default=None,
        help="frozen held-out manifest JSON (default: bench/held_out.json)",
    )
    ap.add_argument(
        "--verify-manifest",
        metavar="DIR",
        help="verify that DIR contains every frozen held-out rig matching --held-out-manifest",
    )
    ap.add_argument(
        "--ranking-claim",
        action="store_true",
        help="enforce frozen held-out coverage (on pair generation) and full held-out gates (on --score-batch)",
    )
    ap.add_argument(
        "--score-batch",
        metavar="DIR",
        help="verify a batch folder's run_manifest.json, .key.json, and verdict.json files",
    )
    ap.add_argument("runs", nargs="*", help="NAME=a.glb,b.glb")
    a = ap.parse_args()

    manifest_explicit = a.held_out_manifest is not None
    if a.held_out_manifest is None:
        a.held_out_manifest = DEFAULT_MANIFEST

    modes = sum(bool(x) for x in (a.verify_manifest, a.score, a.score_batch, a.out))
    if modes > 1:
        ap.error("choose only one of --verify-manifest, --score, --score-batch, or --out")

    if a.verify_manifest:
        return cmd_verify_manifest(a.verify_manifest, a.held_out_manifest)

    if a.score_batch:
        return cmd_score_batch(a.score_batch, a.held_out_manifest, manifest_explicit, a.ranking_claim)

    if a.score:
        if not a.answers:
            ap.error("--score needs --answers")
        w = os.path.abspath(a.weights)
        cmd = [w, "blind-score", a.score, "--answers", a.answers]
        if a.key:
            cmd += ["--key", a.key]
        return subprocess.run(cmd).returncode

    return cmd_generate_pairs(a, ap, manifest_explicit)


if __name__ == "__main__":
    sys.exit(main())
