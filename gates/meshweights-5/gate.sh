#!/usr/bin/env bash
# Gate for meshweights-5 (game-tools docs/plans/meshweights.md): creature clips in the check.
#  - `weights check --motion` takes creature-gait / creature-clips GLBs (walk, trot, lunge,
#    idle, attack) without requiring `--motion-clips` to override humanoid GAME_CLIPS;
#  - works for quadrupeds, multi-legged creatures (Body::Legged), and same-skeleton custom
#    creatures (Body::Other);
#  - clip tears are reported in the "game clips" headline section and `blind::planted_bad`
#    flags a planted left/right leg swap on creature clips;
#  - scope: only rust/core/src/{lib,motion,blind,poses}.rs and rust/cli/src/main.rs changed;
#  - cargo fmt --all --check; release build of all targets with zero warnings; full
#    cargo test --release --no-fail-fast including creature_clips; bench/fix_golden.sh --check
#    byte-identical (no humanoid regressions).
# Builds (release, about 6 cores). Prints PASS/FAIL per check, ends with GATE PASS or GATE FAIL.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
SLUG=meshweights-5
FAILS=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; FAILS=$((FAILS + 1)); }
show() { sed 's/^/      /' | head -n "${2:-20}"; }
T=$(mktemp -d)
BASE=$(git log -1 --format=%H -- gates/$SLUG)
[ -n "$BASE" ] || { fail "cannot find gate commit"; BASE=HEAD; }

echo "== scope"
mine=$(for c in $(git log --format=%H --grep "^carbon $SLUG: " "$BASE..HEAD"); do git diff-tree --no-commit-id --name-only -r "$c"; done)
touched=$( (echo "$mine"; git status --porcelain | cut -c4-) | grep -v '^$' | sort -u)
other=$(echo "$touched" | grep -v -E '^rust/core/src/(lib|motion|blind|poses)\.rs$|^rust/cli/src/main\.rs$')
if [ -z "$other" ]; then pass "only allowed motion/check/blind files changed"; else fail "scope violation: other files changed"; echo "$other" | show; fi
nb=$(git grep -c '#\[test\]' "$BASE" -- rust | awk -F: '{s+=$NF} END {print s+0}')
wt=$(grep -rc '#\[test\]' rust --include='*.rs' | grep -v '/target' | awk -F: '{s+=$NF} END {print s+0}')
ni=$(grep -rc '#\[ignore' rust --include='*.rs' | grep -v '/target' | awk -F: '{s+=$NF} END {print s+0}')
nib=$(git grep -c '#\[ignore' "$BASE" -- rust | awk -F: '{s+=$NF} END {print s+0}')
[ "$wt" -ge "$nb" ] && [ "$ni" -le "$nib" ] && pass "tests $wt (base $nb), ignored $ni (base $nib)" || fail "test count or ignored count regressed"

echo "== build"
command -v cargo >/dev/null 2>&1 || { fail "no cargo"; echo "GATE FAIL ($FAILS)"; exit 1; }
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/rust/target}/gate-$(git rev-parse --short=12 HEAD)-$$"
trap 'rm -rf "$T" "$CARGO_TARGET_DIR"' EXIT
if (cd rust && cargo fmt --all --check) > "$T/fmt" 2>&1; then pass "cargo fmt --all --check"; else fail "cargo fmt --all --check failed"; show 8 < "$T/fmt"; fi
(cd rust && cargo build --release --all-targets) > "$T/build" 2>&1
bc=$?; nw=$(grep -cE '^warning' "$T/build")
if [ $bc -eq 0 ] && [ "$nw" -eq 0 ]; then pass "release build of every target, zero warnings"; else fail "release build failed or had warnings"; grep -E '^(warning|error)' -A4 "$T/build" | show 20; fi

echo "== creature_clips"
(cd rust && cargo test --release --test creature_clips --no-fail-fast) > "$T/cclips" 2>&1
cc=$?
if [ $cc -eq 0 ]; then pass "creature_clips integration tests passed"; else fail "creature_clips integration tests failed"; grep -E '^(test .* FAILED|thread |assertion|    [a-z_]+$)' "$T/cclips" | show 20; fi

echo "== full test suite"
(cd rust && cargo test --release --no-fail-fast) > "$T/test" 2>&1
tc=$?; n=$(grep -E '^test result: ok' "$T/test" | awk '{s+=$4} END {print s+0}'); nf=$(grep -cE '^test result: FAILED' "$T/test")
if [ $tc -eq 0 ] && [ "$nf" -eq 0 ]; then pass "cargo test --release: $n passed"; else fail "cargo test --release failed"; grep -E '^(test .* FAILED|    [a-z_:]+$)' "$T/test" | show 20; fi

echo "== fix golden"
WEIGHTS_BIN="$CARGO_TARGET_DIR/release/weights" bash bench/fix_golden.sh --check > "$T/golden" 2>&1
if tail -1 "$T/golden" | grep -q "^GOLDEN PASS"; then pass "$(tail -1 "$T/golden")"; else fail "bench/fix_golden.sh --check failed"; grep -E "^FAIL|now:" "$T/golden" | show 12; fi

if [ "$FAILS" -eq 0 ]; then echo "GATE PASS"; exit 0; else echo "GATE FAIL ($FAILS)"; exit 1; fi
