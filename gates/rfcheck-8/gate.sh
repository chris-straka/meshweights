#!/usr/bin/env bash
# Gate for rfcheck-8 (game-tools docs/plans/rfcheck.md): meshweights' glbkit pin moved to the
# rfcheck commit that moved the Bevy layer to 0.20 (rfcheck-1, 86abd1e89fdacba39f330b79e4a9201918eafece).
#  - rust/Cargo.toml pins glbkit at that rev, and rust/Cargo.lock resolves it;
#  - scope: the worker changed only rust/Cargo.toml and rust/Cargo.lock;
#  - cargo fmt --all --check; cargo test --release --no-fail-fast (the whole workspace);
#  - bash bench/fix_golden.sh --check: every weights fix output byte-identical to main's.
# Builds (release, about 6 cores; fetches rfcheck from GitHub). Prints PASS/FAIL per check,
# ends with GATE PASS or GATE FAIL.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
SLUG=rfcheck-8
REV=86abd1e89fdacba39f330b79e4a9201918eafece
FAILS=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; FAILS=$((FAILS + 1)); }
show() { sed 's/^/      /' | head -n "${2:-20}"; }
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
BASE=$(git log -1 --format=%H -- gates/$SLUG)
[ -n "$BASE" ] || { fail "cannot find the commit that added the gate"; BASE=HEAD; }

echo "== the pin"
line=$(grep -E '^glbkit\s*=' rust/Cargo.toml)
if echo "$line" | grep -q "rev = \"$REV\""; then pass "rust/Cargo.toml: glbkit at $REV"; else fail "rust/Cargo.toml: $line (want rev $REV)"; fi
if grep -q "rfcheck.git?rev=$REV#$REV" rust/Cargo.lock; then pass "rust/Cargo.lock resolves glbkit at $REV"; else fail "rust/Cargo.lock doesn't resolve glbkit at $REV: $(grep -m1 'rfcheck.git' rust/Cargo.lock)"; fi

echo "== scope"
mine=$(for c in $(git log --format=%H --grep "^carbon $SLUG: " "$BASE..HEAD"); do git diff-tree --no-commit-id --name-only -r "$c"; done)
touched=$( (echo "$mine"; git status --porcelain | cut -c4-) | grep -v '^$' | sort -u)
other=$(echo "$touched" | grep -v -E '^rust/Cargo\.(toml|lock)$')
if [ -z "$other" ]; then pass "only rust/Cargo.toml and rust/Cargo.lock changed"; else fail "other files changed:"; echo "$other" | show; fi

echo "== build and tests"
command -v cargo >/dev/null 2>&1 || { fail "no cargo"; echo "GATE FAIL ($FAILS)"; exit 1; }
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/rust/target}/gate-$(git rev-parse --short=12 HEAD)-$$"
trap 'rm -rf "$T" "$CARGO_TARGET_DIR"' EXIT
if (cd rust && cargo fmt --all --check) > "$T/fmt" 2>&1; then pass "cargo fmt --all --check"; else fail "cargo fmt --all --check"; show 8 < "$T/fmt"; fi
(cd rust && cargo test --release --no-fail-fast) > "$T/test" 2>&1
tc=$?
n=$(grep -E '^test result: ok' "$T/test" | awk '{s+=$4} END {print s+0}')
nf=$(grep -cE '^test result: FAILED' "$T/test")
if [ $tc -eq 0 ] && [ "$nf" -eq 0 ]; then pass "cargo test --release: $n passed"; else fail "cargo test --release: exit $tc, $nf targets failed"; grep -E '^(error|test .* FAILED|    [a-z_:]+$)' "$T/test" | show; fi
WEIGHTS_BIN="$CARGO_TARGET_DIR/release/weights" bash bench/fix_golden.sh --check > "$T/golden" 2>&1
if tail -1 "$T/golden" | grep -q "^GOLDEN PASS"; then pass "$(tail -1 "$T/golden")"; else fail "bench/fix_golden.sh --check"; grep -E "^FAIL|now:" "$T/golden" | show 12; fi

if [ "$FAILS" -eq 0 ]; then echo "GATE PASS"; exit 0; else echo "GATE FAIL ($FAILS)"; exit 1; fi
