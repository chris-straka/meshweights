#!/usr/bin/env python3
"""Same-frame blind pairs for rig claims (plain Python 3, stdlib only).

    python3 bench/blind_bench.py --weights rust/target/release/weights \
        --out DIR NAME=a.glb,b.glb [NAME=a.glb,b.glb ...] [--seed N] [--top N]
    python3 bench/blind_bench.py --score DIR/NAME --answers answers.json

The first form writes one blind run per NAME into DIR/NAME (pairs for
a person to judge, key beside it as NAME.key.json — keep the key away
from the judge). Judge each pair_NN.png as left, right or cant_tell,
put the answers in a JSON file, then the second form scores them:
the control must be cant_tell, the planted bad case must be picked
out and flagged by tears, and any claim where the judge and tears
agree against the old 0-100 score retires that score verdict.
See docs/blind.md.
"""

import argparse
import os
import subprocess
import sys


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--weights", default=os.path.join(os.path.dirname(__file__), "..", "rust", "target", "release", "weights"))
    ap.add_argument("--out")
    ap.add_argument("--seed", default="20261115")
    ap.add_argument("--top", default="6")
    ap.add_argument("--motion")
    ap.add_argument("--score", help="a blind run folder to score instead of making pairs")
    ap.add_argument("--answers", help="judge answers JSON (with --score)")
    ap.add_argument("--key", help="answer key JSON (with --score; default: beside the run)")
    ap.add_argument("runs", nargs="*", help="NAME=a.glb,b.glb")
    a = ap.parse_args()
    w = os.path.abspath(a.weights)

    if a.score:
        if not a.answers:
            ap.error("--score needs --answers")
        cmd = [w, "blind-score", a.score, "--answers", a.answers]
        if a.key:
            cmd += ["--key", a.key]
        return subprocess.run(cmd).returncode

    if not a.out or not a.runs:
        ap.error("making pairs needs --out and NAME=a.glb,b.glb runs")
    os.makedirs(a.out, exist_ok=True)
    rc = 0
    for spec in a.runs:
        name, sep, rest = spec.partition("=")
        left, comma, right = rest.partition(",")
        if not sep or not comma:
            ap.error(f"bad run {spec!r}: want NAME=a.glb,b.glb")
        out = os.path.join(a.out, name)
        cmd = [w, "blind", left, right, "--out", out, "--seed", a.seed, "--top", a.top]
        if a.motion:
            cmd += ["--motion", a.motion]
        print(f"== {name}: {' '.join(cmd)}", flush=True)
        r = subprocess.run(cmd)
        rc = rc or r.returncode
    if rc == 0:
        print("next: judge each pair_NN.png (left/right/cant_tell) without opening any .key.json, "
              "then run with --score RUN --answers answers.json")
    return rc


if __name__ == "__main__":
    sys.exit(main())
