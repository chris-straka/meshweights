#!/usr/bin/env bash
# The sha256 of every `weights fix` output on the test mannequins (`weights fixture all`), per
# method, recorded from main (meshweights-3). A change that must not change behaviour (the
# panic and write-error clean-up, meshweights-4; the fix.rs split, meshweights-8) passes
# `--check` only when every output file is byte-identical and every exit code the same.
#
#   bench/fix_golden.sh --check     compare with bench/fix_golden/<os>-<arch>.txt (the default)
#   bench/fix_golden.sh --record    write that file (only from a main the outputs are right on)
#
# Floats differ in the last bit between macOS arm64 and Linux x86_64, so each platform has its
# own file; a platform with none fails. Builds `weights` (release) unless WEIGHTS_BIN is set.
# Prints PASS/FAIL per case and ends with GOLDEN PASS or GOLDEN FAIL; exit 0 only on a pass.
set -u
cd "$(dirname "$0")/.." || exit 2
mode=${1:---check}
case "$mode" in --check | --record) ;; *) echo "usage: bench/fix_golden.sh [--check|--record]"; exit 2 ;; esac
plat="$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m)"
golden="$PWD/bench/fix_golden/$plat.txt"
if [ -n "${WEIGHTS_BIN:-}" ]; then
  W=$WEIGHTS_BIN
else
  (cd rust && cargo build --release --locked -p weights -q) || { echo "FAIL  cargo build --release -p weights"; echo "GOLDEN FAIL"; exit 1; }
  W="${CARGO_TARGET_DIR:-$PWD/rust/target}/release/weights"
  case "$W" in /*) ;; *) W="$PWD/rust/$W" ;; esac
fi
[ -x "$W" ] || { echo "FAIL  no weights binary at $W"; echo "GOLDEN FAIL"; exit 1; }
if command -v sha256sum >/dev/null 2>&1; then sha() { sha256sum "$1" | cut -d' ' -f1; }; else sha() { shasum -a 256 "$1" | cut -d' ' -f1; }; fi
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
cd "$T" || exit 2
mkdir -p out
"$W" fixture all --out in > /dev/null || { echo "FAIL  weights fixture all"; echo "GOLDEN FAIL"; exit 1; }
# Relative paths throughout: the reports name their input, and must not name this folder.
for f in in/*.glb; do
  n=$(basename "$f" .glb)
  for m in auto smooth geodesic optimize transfer; do
    extra=()
    [ "$m" = transfer ] && extra=(--source in/mannequin_clean.glb)
    "$W" fix "$f" --out "out/$n.$m.glb" --method "$m" ${extra[@]+"${extra[@]}"} > /dev/null 2>&1
    echo "exit $? $n $m"
    for x in "out/$n.$m.glb" "out/$n.$m.report.json"; do
      if [ -f "$x" ]; then echo "$(sha "$x") $x"; else echo "missing $x"; fi
    done
  done
done > now.txt
if [ "$mode" = --record ]; then
  mkdir -p "$(dirname "$golden")"
  cp now.txt "$golden"
  [ -n "${FLEET_ARTIFACTS:-}" ] && cp now.txt "$FLEET_ARTIFACTS/$plat.txt"
  echo "recorded $(wc -l < now.txt | tr -d ' ') lines in bench/fix_golden/$plat.txt"
  exit 0
fi
if [ ! -f "$golden" ]; then echo "FAIL  no golden for $plat (bench/fix_golden/$plat.txt)"; echo "GOLDEN FAIL"; exit 1; fi
fails=0
while IFS= read -r line; do
  if grep -qxF -- "$line" now.txt; then echo "PASS  $line"; else
    echo "FAIL  $line"
    k=$(echo "$line" | awk '{print ($1 == "exit") ? $3 " " $4 : $NF}')
    echo "      now: $(awk -v k="$k" '(($1 == "exit") ? $3 " " $4 : $NF) == k' now.txt)"
    fails=$((fails + 1))
  fi
done < "$golden"
extra=$(grep -vxF -f "$golden" now.txt)
if [ -n "$extra" ]; then echo "FAIL  lines not in the golden:"; echo "$extra" | head -5 | sed 's/^/      /'; fails=$((fails + 1)); fi
if [ "$fails" -eq 0 ]; then echo "GOLDEN PASS ($(wc -l < "$golden" | tr -d ' ') lines)"; exit 0; else echo "GOLDEN FAIL ($fails)"; exit 1; fi
