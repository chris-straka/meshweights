#!/usr/bin/env bash
# Gate for meshweights-8 (game-tools docs/plans/meshweights.md): split fix.rs, no behaviour
# change.
#  - rust/core/src/fix.rs becomes the folder rust/core/src/fix/: mod.rs (at most 1,050 lines),
#    helpers.rs, candidates.rs, picking.rs; every `pub` item fix.rs had is still reachable at
#    `weightforge_core::fix::<name>` (checked by compiling a probe that names each one);
#  - every doc-comment line (`///`, `//!`) and every `// ---` section line of the base's fix.rs
#    is still somewhere in rust/core/src/fix/;
#  - scope: the worker changed only rust/core/src/fix.rs and rust/core/src/fix/ (lib.rs's
#    `mod fix;` needs no change);
#  - cargo fmt --all --check; a release build of every target with zero warnings; the whole
#    cargo test --release --no-fail-fast; no test removed or ignored;
#  - bash bench/fix_golden.sh --check: every weights fix output byte-identical to main's.
# Builds (release, about 6 cores). Prints PASS/FAIL per check, ends with GATE PASS or GATE FAIL.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
SLUG=meshweights-8
FAILS=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; FAILS=$((FAILS + 1)); }
show() { sed 's/^/      /' | head -n "${2:-20}"; }
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
BASE=$(git log -1 --format=%H -- gates/$SLUG)
[ -n "$BASE" ] || { fail "cannot find the commit that added the gate"; BASE=HEAD; }
git show "$BASE:rust/core/src/fix.rs" > "$T/base_fix.rs" || { fail "no fix.rs at the base"; }

echo "== the split"
if [ -e rust/core/src/fix.rs ]; then fail "rust/core/src/fix.rs is still there"; else pass "fix.rs became a folder"; fi
for f in mod helpers candidates picking; do [ -f "rust/core/src/fix/$f.rs" ] && pass "fix/$f.rs exists ($(wc -l < rust/core/src/fix/$f.rs | tr -d ' ') lines)" || fail "fix/$f.rs missing"; done
[ -f rust/core/src/fix/mod.rs ] && [ "$(wc -l < rust/core/src/fix/mod.rs)" -le 1050 ] && pass "fix/mod.rs at most 1,050 lines" || fail "fix/mod.rs over 1,050 lines (or missing)"
miss=$(grep -E '^\s*(///|//!)|^// ---' "$T/base_fix.rs" | sed 's/^\s*//' | sort -u | while IFS= read -r l; do cat rust/core/src/fix/*.rs 2>/dev/null | sed 's/^\s*//' | grep -qxF -- "$l" || echo "$l"; done)
if [ -z "$miss" ]; then pass "every doc comment and section line of fix.rs kept"; else fail "$(echo "$miss" | wc -l | tr -d ' ') doc or section lines lost:"; echo "$miss" | show 8; fi

echo "== scope"
mine=$(for c in $(git log --format=%H --grep "^carbon $SLUG: " "$BASE..HEAD"); do git diff-tree --no-commit-id --name-only -r "$c"; done)
touched=$( (echo "$mine"; git status --porcelain | cut -c4-) | grep -v '^$' | sort -u)
other=$(echo "$touched" | grep -v -E '^rust/core/src/fix(\.rs|/.*)$')
if [ -z "$other" ]; then pass "only fix.rs / fix/ changed"; else fail "other files changed:"; echo "$other" | show; fi
nt=$(git grep -c '#\[test\]' -- rust | awk -F: '{s+=$NF} END {print s+0}'); nb=$(git grep -c '#\[test\]' "$BASE" -- rust | awk -F: '{s+=$NF} END {print s+0}')
wt=$(grep -rc '#\[test\]' rust --include='*.rs' | grep -v '/target' | awk -F: '{s+=$NF} END {print s+0}')
ni=$(grep -rc '#\[ignore' rust --include='*.rs' | grep -v '/target' | awk -F: '{s+=$NF} END {print s+0}'); nib=$(git grep -c '#\[ignore' "$BASE" -- rust | awk -F: '{s+=$NF} END {print s+0}')
[ "$wt" -ge "$nb" ] && [ "$ni" -le "$nib" ] && pass "tests $wt (base $nb), ignored $ni (base $nib)" || fail "tests $wt (base $nb), ignored $ni (base $nib)"

echo "== build"
command -v cargo >/dev/null 2>&1 || { fail "no cargo"; echo "GATE FAIL ($FAILS)"; exit 1; }
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/rust/target}/gate-$(git rev-parse --short=12 HEAD)-$$"
trap 'rm -rf "$T" "$CARGO_TARGET_DIR" rust/core/tests/zz_fix_paths_probe.rs' EXIT
if (cd rust && cargo fmt --all --check) > "$T/fmt" 2>&1; then pass "cargo fmt --all --check"; else fail "cargo fmt --all --check"; show 8 < "$T/fmt"; fi
# The probe: every pub item of the base's fix.rs, named by path.
python3 - "$T/base_fix.rs" > rust/core/tests/zz_fix_paths_probe.rs <<'PY'
import re, sys
s = open(sys.argv[1]).read()
names = sorted(set(re.findall(r"^pub (?:fn|struct|enum|const|type|mod) ([A-Za-z_][A-Za-z0-9_]*)", s, re.M)))
print("// written by gates/meshweights-8/gate.sh; removed after the run")
print("#[allow(unused_imports)]")
print("use weightforge_core::fix::{" + ", ".join(names) + "};")
print("#[test]\nfn zz_fix_paths_probe() {}")
PY
(cd rust && cargo build --release --all-targets) > "$T/build" 2>&1
bc=$?; nw=$(grep -cE '^warning' "$T/build")
if [ $bc -eq 0 ] && [ "$nw" -eq 0 ]; then pass "release build of every target, zero warnings; every pub item of fix.rs still at weightforge_core::fix"; else fail "build: exit $bc, $nw warnings"; grep -E '^(warning|error)' -A4 "$T/build" | show 20; fi
rm -f rust/core/tests/zz_fix_paths_probe.rs
(cd rust && cargo test --release --no-fail-fast) > "$T/test" 2>&1
tc=$?; n=$(grep -E '^test result: ok' "$T/test" | awk '{s+=$4} END {print s+0}'); nf=$(grep -cE '^test result: FAILED' "$T/test")
if [ $tc -eq 0 ] && [ "$nf" -eq 0 ]; then pass "cargo test --release: $n passed"; else fail "cargo test: exit $tc, $nf targets failed"; grep -E '^(test .* FAILED|    [a-z_:]+$)' "$T/test" | show; fi
WEIGHTS_BIN="$CARGO_TARGET_DIR/release/weights" bash bench/fix_golden.sh --check > "$T/golden" 2>&1
if tail -1 "$T/golden" | grep -q "^GOLDEN PASS"; then pass "$(tail -1 "$T/golden")"; else fail "bench/fix_golden.sh --check"; grep -E "^FAIL|now:" "$T/golden" | show 12; fi

if [ "$FAILS" -eq 0 ]; then echo "GATE PASS"; exit 0; else echo "GATE FAIL ($FAILS)"; exit 1; fi
