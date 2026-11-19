#!/usr/bin/env bash
# Gate for meshweights-4 (game-tools docs/plans/meshweights.md): no panics, no silent writes.
#  - scope: the worker changed only rust/core/src/ and rust/cli/src/ (tests, bench, goldens,
#    gates, Cargo files and docs as they were);
#  - no `partial_cmp(..).unwrap()` left in rust/core/src or rust/cli/src (one line or split
#    over lines), and no `let _ = std::fs::write` in rust/cli/src;
#  - no test removed or ignored;
#  - cargo fmt --all --check; cargo build --release --all-targets with zero warnings;
#  - rust/cli/tests/robust_input.rs: all 8 tests pass (NaN position and bind matrix are exit 2
#    with a message, never a hang or panic; findings.json and verdict.json write errors are loud);
#  - the whole cargo test --release --no-fail-fast passes;
#  - bash bench/fix_golden.sh --check: every weights fix output byte-identical to main's.
# Builds (release, about 6 cores). Prints PASS/FAIL per check, ends with GATE PASS or GATE FAIL.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
SLUG=meshweights-4
FAILS=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; FAILS=$((FAILS + 1)); }
show() { sed 's/^/      /' | head -n "${2:-30}"; }
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
BASE=$(git log -1 --format=%H -- gates/$SLUG)
[ -n "$BASE" ] || { fail "cannot find the commit that added the gate"; BASE=HEAD; }

echo "== scope"
mine=$(for c in $(git log --format=%H --grep "^carbon $SLUG: " "$BASE..HEAD"); do git diff-tree --no-commit-id --name-only -r "$c"; done)
other=$( (echo "$mine"; git status --porcelain | cut -c4-) | grep -v -E '^((rust/core/src|rust/cli/src)/.*)?$' | sort -u)
if [ -z "$other" ]; then pass "only rust/core/src/ and rust/cli/src/ changed"; else fail "other files changed:"; echo "$other" | show; fi

echo "== no panicking float sorts, no swallowed writes"
left=$(python3 - <<'PY'
import glob, re
pat = re.compile(r"partial_cmp\([^;{}]*?\)\s*\.unwrap\(\)", re.S)
for f in sorted(glob.glob("rust/core/src/**/*.rs", recursive=True) + glob.glob("rust/cli/src/**/*.rs", recursive=True)):
    s = open(f).read()
    for m in pat.finditer(s):
        print(f"{f}:{s.count(chr(10), 0, m.start()) + 1}")
PY
)
if [ -z "$left" ]; then pass "no partial_cmp(..).unwrap() in rust/core/src, rust/cli/src"; else fail "$(echo "$left" | wc -l | tr -d ' ') partial_cmp(..).unwrap() left:"; echo "$left" | show 10; fi
sw=$(grep -rnE "let _ = (std::)?fs::write" rust/cli/src rust/core/src)
if [ -z "$sw" ]; then pass "no let _ = fs::write"; else fail "swallowed writes left:"; echo "$sw" | show; fi

echo "== tests kept"
nt=$(git grep -c '#\[test\]' -- rust | awk -F: '{s+=$NF} END {print s+0}')
nb=$(git grep -c '#\[test\]' "$BASE" -- rust | awk -F: '{s+=$NF} END {print s+0}')
ni=$(git grep -c '#\[ignore' -- rust | awk -F: '{s+=$NF} END {print s+0}')
nib=$(git grep -c '#\[ignore' "$BASE" -- rust | awk -F: '{s+=$NF} END {print s+0}')
if [ "$nt" -ge "$nb" ] && [ "$ni" -le "$nib" ]; then pass "tests $nt (base $nb), ignored $ni (base $nib)"; else fail "tests $nt (base $nb), ignored $ni (base $nib)"; fi

echo "== format and build"
command -v cargo >/dev/null 2>&1 || { fail "no cargo on this machine"; echo "GATE FAIL ($FAILS)"; exit 1; }
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/rust/target}/gate-$(git rev-parse --short=12 HEAD)-$$"
trap 'rm -rf "$T" "$CARGO_TARGET_DIR"' EXIT
if (cd rust && cargo fmt --all --check) > "$T/fmt.out" 2>&1; then pass "cargo fmt --all --check"; else fail "cargo fmt --all --check"; show 10 < "$T/fmt.out"; fi
(cd rust && cargo build --release --all-targets) > "$T/build.out" 2>&1
bc=$?
nw=$(grep -cE '^warning' "$T/build.out")
if [ $bc -eq 0 ] && [ "$nw" -eq 0 ]; then pass "cargo build --release --all-targets, zero warnings"; else fail "cargo build --release --all-targets: exit $bc, $nw warnings"; grep -E '^(warning|error)' -A4 "$T/build.out" | show 20; fi

echo "== robust_input"
(cd rust && cargo test --release -p weights --test robust_input -- --test-threads=4) > "$T/robust.out" 2>&1
for t in nan_position_is_an_input_error nan_bind_matrix_is_an_input_error nan_weight_fails_the_rig fix_refuses_nan_and_writes_nothing \
  clean_inputs_still_work show_fails_when_findings_json_cant_be_written blind_score_fails_when_verdict_json_cant_be_written \
  fix_and_dump_report_unwritable_outputs; do
  if grep -qE "^test $t \.\.\. ok$" "$T/robust.out"; then pass "$t"; else fail "$t"; awk -v t="---- $t stdout ----" '$0 == t {p = 1} p' "$T/robust.out" | show 8; fi
done

echo "== the whole suite"
(cd rust && cargo test --release --no-fail-fast) > "$T/test.out" 2>&1
tc=$?
nfail=$(grep -E '^test result: FAILED' -c "$T/test.out")
npass=$(grep -E '^test result: ok' "$T/test.out" | awk '{s+=$4} END {print s+0}')
if [ $tc -eq 0 ] && [ "$nfail" -eq 0 ]; then pass "cargo test --release --no-fail-fast: $npass passed"; else fail "cargo test --release --no-fail-fast: exit $tc, $nfail targets failed"; grep -E '^(test .* FAILED|failures:|    [a-z_:]+$)' "$T/test.out" | show 20; fi

echo "== fix outputs unchanged"
WEIGHTS_BIN="$CARGO_TARGET_DIR/release/weights" bash bench/fix_golden.sh --check > "$T/golden.out" 2>&1
if tail -1 "$T/golden.out" | grep -q "^GOLDEN PASS"; then pass "$(tail -1 "$T/golden.out")"; else fail "bench/fix_golden.sh --check"; grep -E "^FAIL|now:" "$T/golden.out" | show 12; fi

if [ "$FAILS" -eq 0 ]; then echo "GATE PASS"; exit 0; else echo "GATE FAIL ($FAILS)"; exit 1; fi
