#!/usr/bin/env bash
# Gate for meshweights-6 (tooling step): kind-specific motion flags in bench/blind_bench.py
# so mixed human+creature --ranking-claim batches apply --human-motion to humans while
# letting creatures use their own gait clips (or --creature-motion when given).
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
SLUG=meshweights-6
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

echo "== syntax"
if python3 -m py_compile bench/blind_bench.py > "$T/pyc" 2>&1; then pass "bench/blind_bench.py syntax valid"; else fail "bench/blind_bench.py syntax check failed"; show 10 < "$T/pyc"; fi

echo "== tests"
if python3 gates/meshweights-2/test_blind_bench.py > "$T/t2" 2>&1; then pass "meshweights-2 tests still pass"; else fail "meshweights-2 tests failed"; show 20 < "$T/t2"; fi
if python3 gates/meshweights-6/test_held_out_motion.py > "$T/t6" 2>&1; then pass "meshweights-6 kind-specific motion tests passed"; else fail "meshweights-6 kind-specific motion tests failed"; show 20 < "$T/t6"; fi

if [ "$FAILS" -eq 0 ]; then echo "GATE PASS"; exit 0; else echo "GATE FAIL ($FAILS)"; exit 1; fi
