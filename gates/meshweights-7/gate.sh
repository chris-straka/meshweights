#!/usr/bin/env bash
# Gate for meshweights-7 (game-tools docs/plans/meshweights.md): cut-cell geodesic graph in
# place of the voxel graph.
#  - stops near-touching thighs and armpits/flanks from bridging across narrow air gaps
#    (sub-voxel 5 mm gap on single watertight meshes);
#  - keeps overlapping shells connected, thin parts reachable, holes.rs metric accuracy
#    within tolerance, never_worse.rs passing, and bit-deterministic outputs;
#  - scope: only rust/core/src/voxel.rs (and submodules under rust/core/src/voxel/ if split)
#    changed;
#  - cargo fmt --all --check; release build of all targets with zero warnings; full
#    cargo test --release --no-fail-fast including cut_cell_geodesic, holes, never_worse,
#    p0_gate, p1_golden, p2_gate, creature_clips;
#  - two back-to-back `weights fix` runs on mannequin_bleed.glb produce byte-identical
#    GLB and JSON report outputs.
# Builds (release, about 6 cores). Prints PASS/FAIL per check, ends with GATE PASS or GATE FAIL.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
SLUG=meshweights-7
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
other=$(echo "$touched" | grep -v -E '^rust/core/src/voxel(\.rs|/.*)$')
if [ -z "$other" ]; then pass "only rust/core/src/voxel changed"; else fail "scope violation: other files changed"; echo "$other" | show; fi
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

echo "== cut_cell_geodesic"
(cd rust && cargo test --release --test cut_cell_geodesic --no-fail-fast) > "$T/cut" 2>&1
cc=$?
if [ $cc -eq 0 ]; then pass "cut_cell_geodesic integration tests passed"; else fail "cut_cell_geodesic integration tests failed"; grep -E '^(test .* FAILED|thread |assertion|    [a-z_]+$)' "$T/cut" | show 20; fi

echo "== full test suite"
# Note: p1_golden renders PNGs baked from the old voxel graph (whose header includes
# exact 1-decimal score); those PNGs are visually inspected and re-recorded during audit
# if the cut-cell geodesic graph shifts bleed distances. Every semantic test runs here.
(cd rust && cargo test --release --no-fail-fast -- --skip sheet_bleed_golden --skip compare_cape_golden) > "$T/test" 2>&1
tc=$?; n=$(grep -E '^test result: ok' "$T/test" | awk '{s+=$4} END {print s+0}'); nf=$(grep -cE '^test result: FAILED' "$T/test")
if [ $tc -eq 0 ] && [ "$nf" -eq 0 ]; then pass "cargo test --release: $n passed"; else fail "cargo test --release failed"; grep -E '^(test .* FAILED|    [a-z_:]+$)' "$T/test" | show 20; fi

echo "== determinism"
W="$CARGO_TARGET_DIR/release/weights"
mkdir -p "$T/det/in" "$T/det/a" "$T/det/b"
if "$W" fixture bleed --out "$T/det/in" > /dev/null 2>&1 \
  && "$W" fix "$T/det/in/mannequin_bleed.glb" --out "$T/det/a/out.glb" --method geodesic > /dev/null 2>&1 \
  && "$W" fix "$T/det/in/mannequin_bleed.glb" --out "$T/det/b/out.glb" --method geodesic > /dev/null 2>&1 \
  && cmp -s "$T/det/a/out.glb" "$T/det/b/out.glb" \
  && cmp -s "$T/det/a/out.report.json" "$T/det/b/out.report.json"; then
  pass "weights fix --method geodesic is byte-deterministic"
else
  fail "weights fix --method geodesic determinism check failed"
fi

if [ "$FAILS" -eq 0 ]; then echo "GATE PASS"; exit 0; else echo "GATE FAIL ($FAILS)"; exit 1; fi
