#!/usr/bin/env python3
"""Calibrate `weights` tears on hand-made artist animals vs generated creatures.

Plain Python 3 (stdlib only).

Mode 1 — Measure & generate blind game-camera pairs:
    python3 bench/creature_calib.py \\
        [--weights PATH] \\
        [--artists-dir DIR] [--artist NAME=PATH ...] \\
        [--generated-dir DIR] [--generated NAME=PATH ...] \\
        --out DIR [--seed N] [--top N] [--creature-motion PATH] \\
        [--gait] [--require-raw]

Mode 2 — Score blind answers & emit final calibration verdict:
    python3 bench/creature_calib.py \\
        --score DIR [--weights PATH] [--min-agreement 0.80]
"""

import argparse
import json
import math
import os
import shutil
import statistics
import subprocess
import sys

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.normpath(os.path.join(SCRIPT_DIR, ".."))
DEFAULT_WEIGHTS = os.path.join(REPO_ROOT, "rust", "target", "release", "weights")
TEARS_EPS = 1e-9


def resolve_weights_bin(spec: str) -> str:
    if os.sep not in spec and (os.altsep is None or os.altsep not in spec):
        found = shutil.which(spec)
        if found:
            return os.path.abspath(found)
    return os.path.abspath(spec)


def is_finite_number(val) -> bool:
    return isinstance(val, (int, float)) and not isinstance(val, bool) and math.isfinite(val)


def is_valid_name(name: str) -> bool:
    return bool(name) and "/" not in name and "\\" not in name and name not in (".", "..")


def parse_named_spec(spec: str, flag: str) -> tuple:
    name, sep, path = spec.partition("=")
    if not sep or not is_valid_name(name) or not path:
        raise ValueError(f"invalid {flag} value {spec!r}: expected NAME=PATH")
    if not os.path.isfile(path):
        raise ValueError(f"{flag} {name!r}: file not found: {path}")
    return name, os.path.abspath(path)


def discover_creatures(
    dirs: list,
    explicit: list,
    flag: str,
    allow_truth_subdir: bool = False,
    subdir_candidates: tuple = (),
) -> list:
    items = []
    seen = set()
    if not subdir_candidates and allow_truth_subdir:
        subdir_candidates = ("truth.glb",)

    def add(name: str, path: str) -> None:
        if not is_valid_name(name):
            raise ValueError(f"invalid creature name {name!r}")
        if name in seen:
            raise ValueError(f"duplicate {flag} creature name {name!r}")
        seen.add(name)
        items.append((name, path))

    for d in dirs:
        if not os.path.isdir(d):
            raise ValueError(f"directory does not exist: {d}")
        for entry in sorted(os.listdir(d)):
            full = os.path.join(d, entry)
            if os.path.isfile(full) and entry.lower().endswith(".glb"):
                stem = os.path.splitext(entry)[0]
                add(stem, os.path.abspath(full))
            elif subdir_candidates and os.path.isdir(full):
                for sub_name in subdir_candidates:
                    cand = os.path.join(full, sub_name)
                    if os.path.isfile(cand):
                        add(entry, os.path.abspath(cand))
                        break

    for spec in explicit:
        name, path = parse_named_spec(spec, flag)
        add(name, path)

    return items


def run_weights_check(
    weights_bin: str,
    glb_path: str,
    creature_motion: str = None,
    use_gait: bool = False,
) -> dict:
    cmd = [weights_bin, "check", glb_path, "--json"]
    if creature_motion:
        cmd.extend(["--motion", creature_motion])
    elif use_gait:
        cmd.append("--gait")
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True)
    except OSError as exc:
        raise RuntimeError(f"failed to run weights check on {glb_path}: {exc}") from exc

    if proc.returncode not in (0, 1):
        err_tail = (proc.stderr or proc.stdout or "").strip()
        raise RuntimeError(
            f"weights check failed (exit {proc.returncode}) on {glb_path}: {err_tail}"
        )

    try:
        doc = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(
            f"weights check returned invalid JSON on {glb_path}: {exc}"
        ) from exc

    if not isinstance(doc, dict):
        raise RuntimeError(f"weights check JSON on {glb_path} is not an object")

    headline = doc.get("headline")
    if not isinstance(headline, list) or not headline:
        raise RuntimeError(f"weights check JSON on {glb_path} missing non-empty 'headline'")

    poses_tears = None
    clips_tears = None
    for sec in headline:
        if not isinstance(sec, dict):
            raise RuntimeError(f"weights check headline entry on {glb_path} is not an object")
        sec_name = sec.get("name")
        tears = sec.get("tears_pct")
        if sec_name == "poses":
            if not is_finite_number(tears):
                raise RuntimeError(
                    f"weights check poses tears_pct invalid on {glb_path}: {tears!r}"
                )
            poses_tears = float(tears)
        elif sec_name == "game clips":
            if not is_finite_number(tears):
                raise RuntimeError(
                    f"weights check game clips tears_pct invalid on {glb_path}: {tears!r}"
                )
            clips_tears = float(tears)

    if poses_tears is None:
        raise RuntimeError(f"weights check JSON on {glb_path} missing 'poses' headline section")

    score = doc.get("score")
    if not is_finite_number(score):
        raise RuntimeError(f"weights check JSON on {glb_path} has invalid 'score': {score!r}")

    return {
        "poses_tears_pct": poses_tears,
        "clips_tears_pct": clips_tears,
        "score": float(score),
        "class": doc.get("class"),
        "pass": bool(doc.get("pass", False)),
    }


def run_weights_fix(
    weights_bin: str,
    glb_path: str,
    fixed_path: str,
    creature_motion: str = None,
    has_own_clips: bool = False,
    use_gait: bool = False,
) -> str:
    os.makedirs(os.path.dirname(os.path.abspath(fixed_path)), exist_ok=True)
    cmd = [weights_bin, "fix", glb_path, "--out", fixed_path]
    if creature_motion:
        cmd.extend(["--motion", creature_motion])
    elif has_own_clips:
        cmd.extend(["--motion", glb_path])
    elif use_gait:
        cmd.append("--gait")

    try:
        proc = subprocess.run(cmd, capture_output=True, text=True)
    except OSError as exc:
        raise RuntimeError(f"failed to run weights fix on {glb_path}: {exc}") from exc

    if proc.returncode not in (0, 1):
        err_tail = (proc.stderr or proc.stdout or "").strip()
        raise RuntimeError(
            f"weights fix failed (exit {proc.returncode}) on {glb_path}: {err_tail}"
        )

    if not os.path.isfile(fixed_path):
        raise RuntimeError(f"weights fix did not produce output file: {fixed_path}")
    return os.path.splitext(fixed_path)[0] + ".report.json"


def run_weights_blind(
    weights_bin: str,
    before_glb: str,
    after_glb: str,
    blind_dir: str,
    seed: int,
    top: int,
    creature_motion: str = None,
    use_gait: bool = False,
) -> str:
    os.makedirs(os.path.dirname(os.path.abspath(blind_dir)), exist_ok=True)
    cmd = [
        weights_bin,
        "blind",
        before_glb,
        after_glb,
        "--out",
        blind_dir,
        "--seed",
        str(seed),
        "--top",
        str(top),
    ]
    if creature_motion:
        cmd.extend(["--motion", creature_motion])
    elif use_gait:
        cmd.append("--gait")

    try:
        proc = subprocess.run(cmd, capture_output=True, text=True)
    except OSError as exc:
        raise RuntimeError(f"failed to run weights blind for {blind_dir}: {exc}") from exc

    if proc.returncode != 0:
        err_tail = (proc.stderr or proc.stdout or "").strip()
        raise RuntimeError(
            f"weights blind failed (exit {proc.returncode}) for {blind_dir}: {err_tail}"
        )

    # Ensure answer key sits beside the judged directory, never inside it.
    for leaked in os.listdir(blind_dir):
        if leaked.endswith(".key.json") or leaked == "answer_key.json":
            raise RuntimeError(
                f"blind answer key {leaked!r} must not live inside judged directory {blind_dir}"
            )

    key_path = os.path.join(
        os.path.dirname(os.path.normpath(blind_dir)),
        os.path.basename(os.path.normpath(blind_dir)) + ".key.json",
    )
    if not os.path.isfile(key_path):
        raise RuntimeError(f"weights blind did not write sibling key file: {key_path}")
    return key_path


def inspect_fix_report(report_path: str) -> dict:
    info = {"verts_changed": None, "mean_l1_change": None, "improved": None}
    if not os.path.isfile(report_path):
        return info
    try:
        with open(report_path, "r", encoding="utf-8") as f:
            doc = json.load(f)
    except (OSError, json.JSONDecodeError):
        return info
    if not isinstance(doc, dict):
        return info
    vc = doc.get("verts_changed")
    if isinstance(vc, int) and not isinstance(vc, bool):
        info["verts_changed"] = vc
    l1 = doc.get("mean_l1_change")
    if is_finite_number(l1):
        info["mean_l1_change"] = float(l1)
    imp = doc.get("improved")
    if isinstance(imp, bool):
        info["improved"] = imp
    return info


def inspect_blind_key(key_path: str) -> dict:
    info = {
        "claim_count": 0,
        "distinct_claims": 0,
        "max_claim_rmse": None,
        "planted_frame": None,
        "planted_rmse": None,
    }
    if not os.path.isfile(key_path):
        return info
    try:
        with open(key_path, "r", encoding="utf-8") as f:
            doc = json.load(f)
    except (OSError, json.JSONDecodeError):
        return info
    if not isinstance(doc, dict):
        return info
    pairs = doc.get("pairs")
    if not isinstance(pairs, list):
        return info
    claim_rmses = []
    for p in pairs:
        if not isinstance(p, dict):
            continue
        cat = p.get("category") or p.get("kind")
        if cat == "claim":
            info["claim_count"] += 1
            rmse = p.get("rmse")
            if is_finite_number(rmse):
                claim_rmses.append(float(rmse))
            if (
                p.get("expected") not in (None, "tie")
                and is_finite_number(rmse)
                and float(rmse) >= 1.0
            ):
                info["distinct_claims"] += 1
        elif cat == "planted":
            frame = p.get("frame")
            if isinstance(frame, str):
                info["planted_frame"] = frame
            rmse = p.get("rmse")
            if is_finite_number(rmse):
                info["planted_rmse"] = float(rmse)
    if claim_rmses:
        info["max_claim_rmse"] = max(claim_rmses)
    return info


def detect_pre_fixed(before: dict, after: dict, report_info: dict, key_info: dict) -> bool:
    verts_changed = report_info.get("verts_changed")
    if verts_changed == 0:
        return True
    claim_count = key_info.get("claim_count", 0)
    distinct_claims = key_info.get("distinct_claims", 0)
    max_claim_rmse = key_info.get("max_claim_rmse")
    if claim_count > 0 and (
        distinct_claims == 0
        or (max_claim_rmse is not None and max_claim_rmse < 1.0)
    ):
        return True
    poses_delta = abs(after["poses_tears_pct"] - before["poses_tears_pct"])
    clips_delta = (
        abs(after["clips_tears_pct"] - before["clips_tears_pct"])
        if before["clips_tears_pct"] is not None and after["clips_tears_pct"] is not None
        else 0.0
    )
    max_delta = max(poses_delta, clips_delta)
    if (before["poses_tears_pct"] > 0.05 or (before["clips_tears_pct"] or 0.0) > 0.05) and max_delta < 0.05:
        if report_info.get("improved") is False or (verts_changed is not None and verts_changed < 5):
            return True
    return False


def evaluate_separation(
    artist_median_poses: float,
    generated_before_median_poses: float,
    artist_median_clips,
    generated_before_median_clips,
) -> tuple:
    reasons = []
    poses_ok = artist_median_poses < generated_before_median_poses
    if not poses_ok:
        reasons.append(
            f"poses separation failed: artist median ({artist_median_poses:.3f}%) "
            f">= generated before median ({generated_before_median_poses:.3f}%)"
        )

    clips_ok = True
    if artist_median_clips is not None and generated_before_median_clips is not None:
        clips_ok = artist_median_clips < generated_before_median_clips
        if not clips_ok:
            reasons.append(
                f"clips separation failed: artist median ({artist_median_clips:.3f}%) "
                f">= generated before median ({generated_before_median_clips:.3f}%)"
            )

    return (poses_ok and clips_ok), reasons


def fmt_pct(val) -> str:
    return "-" if val is None else f"{val:.2f}%"


def fmt_score(val) -> str:
    return "-" if val is None else f"{val:.1f}"


def render_markdown(calib: dict) -> str:
    pre_fixed_count = sum(1 for g in calib["generated"] if g.get("already_fixed"))
    lines = [
        "# Creature Tears Calibration",
        "",
        "## Summary Medians",
        "",
        "| Group | Poses Tears % (median) | Clips Tears % (median) |",
        "|---|---:|---:|",
        f"| Artist | {fmt_pct(calib['artist_median_poses'])} | {fmt_pct(calib['artist_median_clips'])} |",
        f"| Generated (before fix) | {fmt_pct(calib['generated_before_median_poses'])} | {fmt_pct(calib['generated_before_median_clips'])} |",
        f"| Generated (after fix) | {fmt_pct(calib['generated_after_median_poses'])} | {fmt_pct(calib['generated_after_median_clips'])} |",
        "",
        f"- Worse after fix: **{calib['worse_count']}** / {len(calib['generated'])}",
        f"- Raw pre-fix inputs: **{len(calib['generated']) - pre_fixed_count}** / {len(calib['generated'])}",
        "",
        "## Artist Creatures",
        "",
        "| Name | Poses Tears % | Clips Tears % | Score |",
        "|---|---:|---:|---:|",
    ]
    for a in calib["artists"]:
        lines.append(
            f"| {a['name']} | {fmt_pct(a['poses_tears_pct'])} | {fmt_pct(a['clips_tears_pct'])} | {fmt_score(a['score'])} |"
        )

    lines.extend([
        "",
        "## Generated Creatures (Before -> After `weights fix`)",
        "",
        "| Name | Stage | Poses Tears % | Clips Tears % | Score | Verts Changed | Distinct Claims | Max Claim RMSE | Planted Frame | Worse |",
        "|---|---|---:|---:|---:|---:|---:|---:|---|---|",
    ])
    for g in calib["generated"]:
        poses_str = f"{fmt_pct(g['before_poses_tears_pct'])} -> {fmt_pct(g['after_poses_tears_pct'])}"
        if g["before_clips_tears_pct"] is not None or g["after_clips_tears_pct"] is not None:
            clips_str = f"{fmt_pct(g['before_clips_tears_pct'])} -> {fmt_pct(g['after_clips_tears_pct'])}"
        else:
            clips_str = "-"
        score_str = f"{fmt_score(g['before_score'])} -> {fmt_score(g['after_score'])}"
        stage = "pre-fixed" if g.get("already_fixed") else "raw"
        vc = "-" if g.get("verts_changed") is None else str(g["verts_changed"])
        dc = str(g.get("distinct_claims", 0))
        mrmse = "-" if g.get("max_claim_rmse") is None else f"{g['max_claim_rmse']:.2f}"
        pframe = g.get("planted_frame") or "-"
        lines.append(
            f"| {g['name']} | {stage} | {poses_str} | {clips_str} | {score_str} | {vc} | {dc} | {mrmse} | {pframe} | {'YES' if g['worse'] else 'no'} |"
        )

    return "\n".join(lines) + "\n"


def cmd_measure(args) -> int:
    weights_bin = resolve_weights_bin(args.weights)
    if not os.path.isfile(weights_bin):
        print(f"ERROR: weights binary not found: {weights_bin}", file=sys.stderr)
        return 2

    creature_motion = None
    if args.creature_motion:
        creature_motion = os.path.abspath(args.creature_motion)
        if not os.path.isfile(creature_motion):
            print(f"ERROR: --creature-motion file not found: {creature_motion}", file=sys.stderr)
            return 2

    if args.top < 1:
        print(f"ERROR: --top must be >= 1, got {args.top}", file=sys.stderr)
        return 2

    try:
        artist_inputs = discover_creatures(
            args.artists_dir,
            args.artist,
            "--artist",
            subdir_candidates=("truth.glb",),
        )
        generated_inputs = discover_creatures(
            args.generated_dir,
            args.generated,
            "--generated",
            subdir_candidates=("raw.glb", "unfixed.glb"),
        )
    except ValueError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2

    if not artist_inputs:
        print("ERROR: at least 1 artist creature is required (--artists-dir / --artist)", file=sys.stderr)
        return 2
    if not generated_inputs:
        print("ERROR: at least 1 generated creature is required (--generated-dir / --generated)", file=sys.stderr)
        return 2

    out_dir = os.path.abspath(args.out)
    fixed_dir = os.path.join(out_dir, "fixed")
    blind_root = os.path.join(out_dir, "blind")
    os.makedirs(fixed_dir, exist_ok=True)
    os.makedirs(blind_root, exist_ok=True)

    artists = []
    generated = []
    use_gait = bool(args.gait and not creature_motion)

    try:
        for name, path in artist_inputs:
            res = run_weights_check(weights_bin, path, creature_motion, use_gait=use_gait)
            artists.append({
                "name": name,
                "file": os.path.basename(path),
                "poses_tears_pct": res["poses_tears_pct"],
                "clips_tears_pct": res["clips_tears_pct"],
                "score": res["score"],
                "class": res["class"],
            })

        for idx, (name, path) in enumerate(generated_inputs):
            before = run_weights_check(weights_bin, path, creature_motion, use_gait=use_gait)
            fixed_path = os.path.join(fixed_dir, f"{name}_fixed.glb")
            report_path = run_weights_fix(
                weights_bin,
                path,
                fixed_path,
                creature_motion=creature_motion,
                has_own_clips=(before["clips_tears_pct"] is not None),
                use_gait=use_gait,
            )
            after = run_weights_check(weights_bin, fixed_path, creature_motion, use_gait=use_gait)
            blind_dir = os.path.join(blind_root, name)
            key_path = run_weights_blind(
                weights_bin,
                path,
                fixed_path,
                blind_dir,
                seed=args.seed + idx,
                top=args.top,
                creature_motion=creature_motion,
                use_gait=use_gait,
            )
            report_info = inspect_fix_report(report_path)
            key_info = inspect_blind_key(key_path)
            already_fixed = detect_pre_fixed(before, after, report_info, key_info)
            if already_fixed:
                print(
                    f"WARN: generated creature {name!r} appears pre-fixed "
                    f"(verts_changed={report_info['verts_changed']}, "
                    f"distinct_claims={key_info['distinct_claims']}, "
                    f"max_claim_rmse={key_info['max_claim_rmse']}); "
                    f"pass un-fixed raw inputs for visible blind claims",
                    file=sys.stderr,
                )

            poses_worse = after["poses_tears_pct"] > before["poses_tears_pct"] + TEARS_EPS
            clips_worse = (
                before["clips_tears_pct"] is not None
                and after["clips_tears_pct"] is not None
                and after["clips_tears_pct"] > before["clips_tears_pct"] + TEARS_EPS
            )
            worse = poses_worse or clips_worse

            generated.append({
                "name": name,
                "file": os.path.basename(path),
                "fixed_file": os.path.basename(fixed_path),
                "before_poses_tears_pct": before["poses_tears_pct"],
                "after_poses_tears_pct": after["poses_tears_pct"],
                "before_clips_tears_pct": before["clips_tears_pct"],
                "after_clips_tears_pct": after["clips_tears_pct"],
                "before_score": before["score"],
                "after_score": after["score"],
                "class": before["class"],
                "seed": args.seed + idx,
                "worse": worse,
                "verts_changed": report_info["verts_changed"],
                "mean_l1_change": report_info["mean_l1_change"],
                "claim_count": key_info["claim_count"],
                "distinct_claims": key_info["distinct_claims"],
                "max_claim_rmse": key_info["max_claim_rmse"],
                "planted_frame": key_info["planted_frame"],
                "planted_rmse": key_info["planted_rmse"],
                "already_fixed": already_fixed,
            })
    except RuntimeError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2

    artist_median_poses = float(statistics.median([a["poses_tears_pct"] for a in artists]))
    generated_before_median_poses = float(
        statistics.median([g["before_poses_tears_pct"] for g in generated])
    )
    generated_after_median_poses = float(
        statistics.median([g["after_poses_tears_pct"] for g in generated])
    )

    artist_clips_vals = [a["clips_tears_pct"] for a in artists if a["clips_tears_pct"] is not None]
    gen_before_clips_vals = [
        g["before_clips_tears_pct"] for g in generated if g["before_clips_tears_pct"] is not None
    ]
    gen_after_clips_vals = [
        g["after_clips_tears_pct"] for g in generated if g["after_clips_tears_pct"] is not None
    ]

    artist_median_clips = float(statistics.median(artist_clips_vals)) if artist_clips_vals else None
    generated_before_median_clips = (
        float(statistics.median(gen_before_clips_vals)) if gen_before_clips_vals else None
    )
    generated_after_median_clips = (
        float(statistics.median(gen_after_clips_vals)) if gen_after_clips_vals else None
    )

    worse_count = sum(1 for g in generated if g["worse"])
    pre_fixed_names = [g["name"] for g in generated if g["already_fixed"]]
    raw_ok = not pre_fixed_names
    separation_ok, sep_reasons = evaluate_separation(
        artist_median_poses,
        generated_before_median_poses,
        artist_median_clips,
        generated_before_median_clips,
    )
    never_worse_ok = worse_count == 0

    reasons = list(sep_reasons)
    if not never_worse_ok:
        worse_names = [g["name"] for g in generated if g["worse"]]
        reasons.append(
            f"worse tears after fix on {worse_count} generated creature(s): {', '.join(worse_names)}"
        )
    if args.require_raw and not raw_ok:
        reasons.append(
            f"raw pre-fix check failed on pre-fixed creature(s): {', '.join(pre_fixed_names)}"
        )

    calib = {
        "version": 1,
        "seed": args.seed,
        "top": args.top,
        "require_raw": bool(args.require_raw),
        "raw_ok": raw_ok,
        "artist_median_poses": artist_median_poses,
        "generated_before_median_poses": generated_before_median_poses,
        "generated_after_median_poses": generated_after_median_poses,
        "artist_median_clips": artist_median_clips,
        "generated_before_median_clips": generated_before_median_clips,
        "generated_after_median_clips": generated_after_median_clips,
        "worse_count": worse_count,
        "separation_ok": separation_ok,
        "never_worse_ok": never_worse_ok,
        "artists": artists,
        "generated": generated,
    }

    calib_json_path = os.path.join(out_dir, "calib.json")
    calib_md_path = os.path.join(out_dir, "calib.md")
    with open(calib_json_path, "w", encoding="utf-8") as f:
        json.dump(calib, f, indent=2)
        f.write("\n")
    with open(calib_md_path, "w", encoding="utf-8") as f:
        f.write(render_markdown(calib))

    print(
        f"calib poses median: artist={artist_median_poses:.2f}% "
        f"gen_before={generated_before_median_poses:.2f}% "
        f"gen_after={generated_after_median_poses:.2f}% "
        f"(worse={worse_count})"
    )
    if artist_median_clips is not None or generated_before_median_clips is not None:
        print(
            f"calib clips median: artist={fmt_pct(artist_median_clips)} "
            f"gen_before={fmt_pct(generated_before_median_clips)} "
            f"gen_after={fmt_pct(generated_after_median_clips)}"
        )

    if reasons:
        for reason in reasons:
            print(f"FAIL: {reason}", file=sys.stderr)
        return 1

    print(f"PASS measure: wrote {calib_json_path} and {calib_md_path}")
    return 0


def cmd_score(args) -> int:
    score_dir = os.path.abspath(args.score)
    min_agreement = args.min_agreement
    if not is_finite_number(min_agreement) or not (0.0 <= min_agreement <= 1.0):
        print(f"ERROR: --min-agreement must be in [0.0, 1.0], got {min_agreement!r}", file=sys.stderr)
        return 2

    calib_path = os.path.join(score_dir, "calib.json")
    if not os.path.isfile(calib_path):
        print(f"ERROR: calib.json not found in {score_dir}", file=sys.stderr)
        return 2

    try:
        with open(calib_path, "r", encoding="utf-8") as f:
            calib = json.load(f)
    except (OSError, json.JSONDecodeError) as exc:
        print(f"ERROR: failed to read {calib_path}: {exc}", file=sys.stderr)
        return 2

    if not isinstance(calib, dict):
        print(f"ERROR: invalid {calib_path}: expected JSON object", file=sys.stderr)
        return 2

    artist_median_poses = calib.get("artist_median_poses")
    gen_before_median_poses = calib.get("generated_before_median_poses")
    artist_median_clips = calib.get("artist_median_clips")
    gen_before_median_clips = calib.get("generated_before_median_clips")
    worse_count = calib.get("worse_count")
    generated = calib.get("generated")

    if not is_finite_number(artist_median_poses) or not is_finite_number(gen_before_median_poses):
        print(f"ERROR: invalid pose medians in {calib_path}", file=sys.stderr)
        return 2
    if artist_median_clips is not None and not is_finite_number(artist_median_clips):
        print(f"ERROR: invalid artist_median_clips in {calib_path}", file=sys.stderr)
        return 2
    if gen_before_median_clips is not None and not is_finite_number(gen_before_median_clips):
        print(f"ERROR: invalid generated_before_median_clips in {calib_path}", file=sys.stderr)
        return 2
    if not isinstance(worse_count, int) or isinstance(worse_count, bool) or worse_count < 0:
        print(f"ERROR: invalid worse_count in {calib_path}", file=sys.stderr)
        return 2
    if not isinstance(generated, list) or not generated:
        print(f"ERROR: invalid or empty generated list in {calib_path}", file=sys.stderr)
        return 2

    separation_ok, reasons = evaluate_separation(
        float(artist_median_poses),
        float(gen_before_median_poses),
        float(artist_median_clips) if artist_median_clips is not None else None,
        float(gen_before_median_clips) if gen_before_median_clips is not None else None,
    )

    # Check both recorded worse_count and per-creature worse flags/numbers.
    computed_worse = 0
    for entry in generated:
        if not isinstance(entry, dict):
            print(f"ERROR: malformed generated entry in {calib_path}", file=sys.stderr)
            return 2
        bp = entry.get("before_poses_tears_pct")
        ap = entry.get("after_poses_tears_pct")
        bc = entry.get("before_clips_tears_pct")
        ac = entry.get("after_clips_tears_pct")
        entry_worse = bool(entry.get("worse", False))
        if is_finite_number(bp) and is_finite_number(ap) and float(ap) > float(bp) + TEARS_EPS:
            entry_worse = True
        if is_finite_number(bc) and is_finite_number(ac) and float(ac) > float(bc) + TEARS_EPS:
            entry_worse = True
        if entry_worse:
            computed_worse += 1

    effective_worse = max(worse_count, computed_worse)
    never_worse_ok = effective_worse == 0
    if not never_worse_ok:
        reasons.append(f"never-worse failed: {effective_worse} generated creature(s) worsened after fix")

    weights_bin = resolve_weights_bin(args.weights)
    blind_gates_ok = True
    total_claims = 0
    total_agrees = 0
    blind_runs = []

    for entry in generated:
        name = entry.get("name")
        if not isinstance(name, str) or not is_valid_name(name):
            blind_gates_ok = False
            reasons.append(f"invalid generated creature name in calib.json: {name!r}")
            continue

        blind_dir = os.path.join(score_dir, "blind", name)
        verdict_path = os.path.join(blind_dir, "verdict.json")
        answers_path = os.path.join(blind_dir, "answers.json")

        if not os.path.isfile(verdict_path):
            if os.path.isfile(answers_path):
                if not os.path.isfile(weights_bin):
                    print(f"ERROR: weights binary not found: {weights_bin}", file=sys.stderr)
                    return 2
                cmd = [weights_bin, "blind-score", blind_dir, "--answers", answers_path]
                try:
                    proc = subprocess.run(cmd, capture_output=True, text=True)
                except OSError as exc:
                    blind_gates_ok = False
                    reasons.append(f"{name}: failed to run weights blind-score: {exc}")
                    continue
                if proc.returncode not in (0, 1):
                    err_tail = (proc.stderr or proc.stdout or "").strip()
                    blind_gates_ok = False
                    reasons.append(
                        f"{name}: weights blind-score crashed (exit {proc.returncode}): {err_tail}"
                    )
                    continue
            else:
                blind_gates_ok = False
                reasons.append(f"{name}: missing verdict.json and answers.json in {blind_dir}")
                continue

        if not os.path.isfile(verdict_path):
            blind_gates_ok = False
            reasons.append(f"{name}: verdict.json was not produced in {blind_dir}")
            continue

        try:
            with open(verdict_path, "r", encoding="utf-8") as f:
                verdict = json.load(f)
        except (OSError, json.JSONDecodeError) as exc:
            blind_gates_ok = False
            reasons.append(f"{name}: failed to parse verdict.json: {exc}")
            continue

        if not isinstance(verdict, dict):
            blind_gates_ok = False
            reasons.append(f"{name}: verdict.json is not a JSON object")
            continue

        failed_flags = [
            flag
            for flag in ("control_ok", "planted_ok", "planted_tears_ok")
            if verdict.get(flag) is not True
        ]
        if failed_flags:
            blind_gates_ok = False
            reasons.append(f"{name}: blind gate failed ({', '.join(failed_flags)})")

        claims_judged = verdict.get("claims_judged")
        judge_agrees_tears = verdict.get("judge_agrees_tears")
        if (
            not isinstance(claims_judged, int)
            or isinstance(claims_judged, bool)
            or claims_judged < 0
            or not isinstance(judge_agrees_tears, int)
            or isinstance(judge_agrees_tears, bool)
            or judge_agrees_tears < 0
            or judge_agrees_tears > claims_judged
        ):
            blind_gates_ok = False
            reasons.append(
                f"{name}: invalid claims_judged/judge_agrees_tears "
                f"({claims_judged!r}, {judge_agrees_tears!r})"
            )
            continue

        total_claims += claims_judged
        total_agrees += judge_agrees_tears
        blind_runs.append({
            "name": name,
            "control_ok": verdict.get("control_ok") is True,
            "planted_ok": verdict.get("planted_ok") is True,
            "planted_tears_ok": verdict.get("planted_tears_ok") is True,
            "claims_judged": claims_judged,
            "judge_agrees_tears": judge_agrees_tears,
        })

    require_raw = bool(calib.get("require_raw"))
    raw_ok = bool(calib.get("raw_ok", True))
    if require_raw and not raw_ok:
        pre_fixed = [g.get("name", "?") for g in generated if g.get("already_fixed")]
        reasons.append(
            f"raw pre-fix check failed: pre-fixed creature(s) present ({', '.join(pre_fixed)})"
        )

    agreement_rate = (total_agrees / total_claims) if total_claims > 0 else 0.0
    if total_claims == 0:
        agreement_ok = False
        if any(g.get("already_fixed") for g in generated):
            reasons.append(
                "no blind claims judged (total_claims == 0; inputs appear pre-fixed — pass un-fixed raw.glb inputs)"
            )
        else:
            reasons.append("no blind claims judged (total_claims == 0)")
    elif agreement_rate < min_agreement:
        agreement_ok = False
        reasons.append(
            f"blind agreement {total_agrees}/{total_claims} ({agreement_rate:.2%}) "
            f"< min_agreement ({min_agreement:.2%})"
        )
    else:
        agreement_ok = True

    passed = separation_ok and never_worse_ok and blind_gates_ok and agreement_ok and (not require_raw or raw_ok)

    verdict_doc = {
        "pass": passed,
        "separation_ok": separation_ok,
        "never_worse_ok": never_worse_ok,
        "blind_gates_ok": blind_gates_ok,
        "raw_ok": raw_ok,
        "total_claims": total_claims,
        "total_agrees": total_agrees,
        "agreement_rate": agreement_rate,
        "min_agreement": min_agreement,
        "reasons": reasons,
        "blind_runs": blind_runs,
    }

    verdict_out_path = os.path.join(score_dir, "calib_verdict.json")
    with open(verdict_out_path, "w", encoding="utf-8") as f:
        json.dump(verdict_doc, f, indent=2)
        f.write("\n")

    status = "PASS" if passed else "FAIL"
    print(
        f"{status} creature calibration: separation={'ok' if separation_ok else 'FAIL'} "
        f"never_worse={'ok' if never_worse_ok else 'FAIL'} "
        f"blind_gates={'ok' if blind_gates_ok else 'FAIL'} "
        f"agreement={total_agrees}/{total_claims} ({agreement_rate:.1%}, min {min_agreement:.1%})"
    )
    if not passed:
        for reason in reasons:
            print(f"  - {reason}", file=sys.stderr)
        return 1
    return 0


def build_parser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(
        description="Calibrate weights tears on artist animals vs generated creatures."
    )
    ap.add_argument(
        "--weights",
        default=DEFAULT_WEIGHTS,
        help="path to weights CLI binary (default: <repo>/rust/target/release/weights)",
    )
    ap.add_argument(
        "--artists-dir",
        action="append",
        default=[],
        metavar="DIR",
        help="directory of artist animal .glb files (or <subdir>/truth.glb); repeatable",
    )
    ap.add_argument(
        "--artist",
        action="append",
        default=[],
        metavar="NAME=PATH",
        help="explicit artist animal GLB; repeatable",
    )
    ap.add_argument(
        "--generated-dir",
        action="append",
        default=[],
        metavar="DIR",
        help="directory of raw generated creature .glb files (or <subdir>/raw.glb); repeatable",
    )
    ap.add_argument(
        "--generated",
        action="append",
        default=[],
        metavar="NAME=PATH",
        help="explicit generated creature GLB; repeatable",
    )
    ap.add_argument(
        "--out",
        metavar="DIR",
        help="output directory for calib.json, calib.md, fixed/, and blind/ (Mode 1)",
    )
    ap.add_argument(
        "--seed",
        type=int,
        default=20261115,
        help="base RNG seed for blind pairs (incremented per generated creature)",
    )
    ap.add_argument(
        "--top",
        type=int,
        default=6,
        help="max blind claim pairs per generated creature",
    )
    ap.add_argument(
        "--creature-motion",
        metavar="PATH",
        help="optional creature-gait motion GLB passed via --motion",
    )
    ap.add_argument(
        "--gait",
        action="store_true",
        help="enable built-in neutral creature gait clips (--gait) when --creature-motion is omitted",
    )
    ap.add_argument(
        "--require-raw",
        action="store_true",
        help="fail Mode 1/2 if any generated creature appears pre-fixed with sub-pixel blind claims",
    )
    ap.add_argument(
        "--score",
        metavar="DIR",
        help="score blind answers in DIR and emit calib_verdict.json (Mode 2)",
    )
    ap.add_argument(
        "--min-agreement",
        type=float,
        default=0.80,
        help="minimum blind judge agreement with tears (default: 0.80)",
    )
    return ap


def main() -> int:
    ap = build_parser()
    args = ap.parse_args()

    if args.score and args.out:
        ap.error("choose either --out (Mode 1) or --score (Mode 2), not both")
    if args.score:
        return cmd_score(args)
    if args.out:
        return cmd_measure(args)

    ap.error("either --out (Mode 1) or --score (Mode 2) is required")
    return 2


if __name__ == "__main__":
    sys.exit(main())
