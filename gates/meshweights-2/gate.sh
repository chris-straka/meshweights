#!/usr/bin/env bash
# Gate for meshweights-2 (game-tools docs/plans/meshweights.md): freeze held-out set +
# bench/blind_bench.py refuses ranking claims without a held-out run.
#  - bench/held_out.json lists 2 humans and 3 creatures by kind and sha256 (zero owner paths);
#  - bench/blind_bench.py implements --ranking-claim, --held-out-manifest, --verify-manifest,
#    and --score-batch, refusing ranking claims unless all 5 frozen held-out rigs are present
#    with matching sha256s, valid control/planted verdicts, zero worse tears, and majority
#    improved tears;
#  - scope: only bench/blind_bench.py (and optionally docs/blind.md) changed by the worker;
#  - plain stdlib Python 3, zero new dependencies, zero private paths or owner names.
# Prints PASS/FAIL per check, ends with GATE PASS or GATE FAIL.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
SLUG=meshweights-2
FAILS=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; FAILS=$((FAILS + 1)); }
show() { sed 's/^/      /' | head -n "${2:-20}"; }
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
BASE=$(git log -1 --format=%H -- gates/$SLUG)
[ -n "$BASE" ] || { fail "cannot find gate commit"; BASE=HEAD; }

echo "== scope"
mine=$(for c in $(git log --format=%H --grep "^carbon $SLUG: " "$BASE..HEAD"); do git diff-tree --no-commit-id --name-only -r "$c"; done)
touched=$( (echo "$mine"; git status --porcelain | cut -c4-) | grep -v '^$' | sort -u)
other=$(echo "$touched" | grep -v -E '^bench/blind_bench\.py$|^docs/blind\.md$')
if [ -z "$other" ]; then pass "only allowed bench/docs files changed"; else fail "scope violation: other files changed"; echo "$other" | show; fi

echo "== syntax and privacy"
if python3 -m py_compile bench/blind_bench.py > "$T/pyc" 2>&1; then pass "bench/blind_bench.py syntax valid"; else fail "bench/blind_bench.py syntax check failed"; show 10 < "$T/pyc"; fi
if grep -qE '/Users/|/home/|/data/|hlh/enemies|hlh/npcs|Games/hlh' bench/held_out.json bench/blind_bench.py; then
  fail "privacy check failed: owner path or asset name found"
else
  pass "no owner paths or private asset names in manifest or script"
fi

echo "== blind_bench tests"
if python3 gates/meshweights-2/test_blind_bench.py > "$T/test" 2>&1; then
  pass "meshweights-2 blind_bench refusal tests passed"
else
  fail "meshweights-2 blind_bench refusal tests failed"
  show 25 < "$T/test"
fi

if [ "$FAILS" -eq 0 ]; then echo "GATE PASS"; exit 0; else echo "GATE FAIL ($FAILS)"; exit 1; fi
