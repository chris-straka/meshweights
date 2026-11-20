#!/usr/bin/env bash
# Gate for meshweights-1 (game-tools docs/plans/meshweights.md): creature tears calibration
# script (`bench/creature_calib.py`).
#  - measures tears on hand-made artist animals vs generated creatures (before & after fix)
#    on poses and creature-gait clips, plus generates blind game-camera pairs;
#  - passes only when artist median < generated median, 0 generated rigs worsen after fix,
#    blind controls & planted cases pass, and blind judge agrees with tears on >= 80% of
#    judged claims (with > 0 claims judged);
#  - fails loudly if `weights check` crashes or returns invalid JSON;
#  - scope: only bench/creature_calib.py (and optionally docs/rig-score.md or docs/blind.md);
#  - stdlib Python 3 only, zero private paths or owner asset names.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
SLUG=meshweights-1
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
other=$(echo "$touched" | grep -v -E '^bench/creature_calib\.py$|^docs/(rig-score|blind)\.md$')
if [ -z "$other" ]; then pass "only allowed bench/docs files changed"; else fail "scope violation: other files changed"; echo "$other" | show; fi

echo "== syntax and privacy"
if [ -f bench/creature_calib.py ] && python3 -m py_compile bench/creature_calib.py > "$T/pyc" 2>&1; then
  pass "bench/creature_calib.py exists and compiles"
else
  fail "bench/creature_calib.py missing or syntax error"
  [ -s "$T/pyc" ] && show 10 < "$T/pyc"
fi
if [ -f bench/creature_calib.py ] && grep -qE '/Users/|/home/|/data/|Games/hlh|hlh/enemies' bench/creature_calib.py; then
  fail "privacy check failed: private path or owner asset name found"
else
  pass "no private paths or owner asset names"
fi

echo "== creature_calib tests"
if [ -f bench/creature_calib.py ] && python3 gates/meshweights-1/test_creature_calib.py > "$T/test" 2>&1; then
  pass "meshweights-1 creature_calib tests passed"
else
  fail "meshweights-1 creature_calib tests failed"
  [ -s "$T/test" ] && show 25 < "$T/test"
fi

if [ "$FAILS" -eq 0 ]; then echo "GATE PASS"; exit 0; else echo "GATE FAIL ($FAILS)"; exit 1; fi
