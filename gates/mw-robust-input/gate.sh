#!/usr/bin/env bash
# Gate for mw-robust-input: robust input handling in rust/core and weights CLI.
# Run as: bash gates/mw-robust-input/gate.sh (from repo root, Linux or macOS, clean checkout).
# Prints PASS <name> or FAIL <name> for every check, runs every check, and ends with
# GATE PASS (exit 0) or GATE FAIL (exit 1).
set -u
export LC_ALL=C
cd "$(git rev-parse --show-toplevel)" || exit 2

SLUG=mw-robust-input
FAILS=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; FAILS=$((FAILS + 1)); }
show() { sed 's/^/      /' | head -n "${2:-25}"; }

T=$(mktemp -d)
CARGO_TARGET_DIR_OWN=""
cleanup() {
  rm -rf "$T"
  if [ -n "$CARGO_TARGET_DIR_OWN" ]; then
    rm -rf "$CARGO_TARGET_DIR_OWN"
  fi
}
trap cleanup EXIT

echo "== tools"
HAVE_CARGO=1
HAVE_PY=1
if command -v cargo >/dev/null 2>&1; then
  pass "tool cargo present"
else
  fail "tool cargo missing"
  HAVE_CARGO=0
fi
if command -v python3 >/dev/null 2>&1; then
  pass "tool python3 present"
else
  fail "tool python3 missing"
  HAVE_PY=0
fi

echo "== scope and protected files"
BASE=$(git log -1 --format=%H -- "gates/$SLUG" 2>/dev/null || true)
if [ -n "$BASE" ]; then
  changed=$( (git diff --name-only "$BASE" HEAD; git status --porcelain | cut -c4-) | grep -v '^$' | sort -u )
else
  BASE=$(git rev-parse HEAD)
  changed=$(git status --porcelain | cut -c4- | grep -v '^$' | grep -v -E '^(_draft|gates/'"$SLUG"')(/.*)?$' | sort -u || true)
fi
other=$(printf '%s\n' "$changed" | grep -v '^$' | grep -v -E '^(rust/core/src/.*|rust/cli/src/.*|rust/core/tests/robust_input\.rs)$' || true)
if [ -z "$other" ]; then
  pass "scope: only rust/core/src/, rust/cli/src/, and rust/core/tests/robust_input.rs changed"
else
  fail "scope: disallowed files changed"
  printf '%s\n' "$other" | show
fi

nb=$(git grep -c '#\[test\]' "$BASE" -- rust | awk -F: '{s+=$NF} END {print s+0}')
wt=$(grep -rc '#\[test\]' rust --include='*.rs' | grep -v '/target' | awk -F: '{s+=$NF} END {print s+0}')
ni=$(grep -rc '#\[ignore' rust --include='*.rs' | grep -v '/target' | awk -F: '{s+=$NF} END {print s+0}')
nib=$(git grep -c '#\[ignore' "$BASE" -- rust | awk -F: '{s+=$NF} END {print s+0}')
if [ "$wt" -ge "$nb" ] && [ "$ni" -le "$nib" ]; then
  pass "tests kept ($wt total vs base $nb, $ni ignored vs base $nib)"
else
  fail "tests regressed ($wt total vs base $nb, $ni ignored vs base $nib)"
fi

if [ -f rust/core/tests/robust_input.rs ]; then
  pass "rust/core/tests/robust_input.rs exists"
else
  fail "rust/core/tests/robust_input.rs missing"
fi

echo "== format, clippy, and release build"
W=""
if [ "$HAVE_CARGO" -eq 1 ]; then
  CARGO_TARGET_DIR_OWN="${CARGO_TARGET_DIR:-$PWD/rust/target}/gate-$SLUG-$(git rev-parse --short=12 HEAD)-$$"
  export CARGO_TARGET_DIR="$CARGO_TARGET_DIR_OWN"

  if (cd rust && cargo fmt --all --check) > "$T/fmt.out" 2>&1; then
    pass "cargo fmt --all --check"
  else
    fail "cargo fmt --all --check"
    show 12 < "$T/fmt.out"
  fi

  if (cd rust && cargo clippy --all-targets --locked -- -D warnings) > "$T/clippy.out" 2>&1; then
    pass "cargo clippy --all-targets --locked -- -D warnings"
  else
    fail "cargo clippy --all-targets --locked -- -D warnings"
    grep -E '^(warning|error)' -A4 "$T/clippy.out" | show 20
  fi

  (cd rust && cargo build --release --locked --all-targets) > "$T/build.out" 2>&1
  bc=$?
  nw=$(grep -cE '^warning' "$T/build.out" || true)
  if [ "$bc" -eq 0 ] && [ "$nw" -eq 0 ]; then
    pass "cargo build --release --locked --all-targets (zero warnings)"
  else
    fail "cargo build --release --locked --all-targets (exit $bc, $nw warnings)"
    grep -E '^(warning|error)' -A4 "$T/build.out" | show 20
  fi

  W="$CARGO_TARGET_DIR/release/weights"
else
  fail "cargo fmt --all --check (no cargo)"
  fail "cargo clippy --all-targets --locked -- -D warnings (no cargo)"
  fail "cargo build --release --locked --all-targets (no cargo)"
fi

echo "== core robust_input tests"
if [ "$HAVE_CARGO" -eq 1 ] && [ -f rust/core/tests/robust_input.rs ]; then
  (cd rust && cargo test --release --locked -p weightforge_core --test robust_input -- --test-threads=4) > "$T/core_robust.out" 2>&1
  crc=$?
  cr_pass=$(grep -E '^test result: ok' "$T/core_robust.out" | awk '{s+=$4} END {print s+0}')
  cr_fail=$(grep -cE '^test result: FAILED' "$T/core_robust.out" || true)
  if [ "$crc" -eq 0 ] && [ "$cr_fail" -eq 0 ] && [ "$cr_pass" -ge 6 ]; then
    pass "core robust_input tests ($cr_pass passed, min 6)"
  else
    fail "core robust_input tests (exit $crc, $cr_pass passed, $cr_fail failed, min 6)"
    show 20 < "$T/core_robust.out"
  fi
else
  fail "core robust_input tests (missing rust/core/tests/robust_input.rs or cargo)"
fi

echo "== cli robust_input tests"
CLI_ROBUST_TESTS="
nan_position_is_an_input_error
nan_bind_matrix_is_an_input_error
nan_weight_fails_the_rig
fix_refuses_nan_and_writes_nothing
clean_inputs_still_work
show_fails_when_findings_json_cant_be_written
blind_score_fails_when_verdict_json_cant_be_written
fix_and_dump_report_unwritable_outputs
malformed_empty_and_invalid_threshold_errors_exit_2
"
if [ "$HAVE_CARGO" -eq 1 ]; then
  (cd rust && cargo test --release --locked -p weights --test robust_input -- --test-threads=4) > "$T/cli_robust.out" 2>&1
  for t in $CLI_ROBUST_TESTS; do
    if grep -qE "^test $t \.\.\. ok$" "$T/cli_robust.out"; then
      pass "cli::$t"
    else
      fail "cli::$t"
      awk -v t="---- $t stdout ----" '$0 == t {p = 1} p' "$T/cli_robust.out" | show 10
    fi
  done
else
  for t in $CLI_ROBUST_TESTS; do
    fail "cli::$t (no cargo)"
  done
fi

echo "== full workspace test suite"
if [ "$HAVE_CARGO" -eq 1 ]; then
  (cd rust && cargo test --release --locked --no-fail-fast) > "$T/test.out" 2>&1
  tc=$?
  nfail=$(grep -cE '^test result: FAILED' "$T/test.out" || true)
  npass=$(grep -E '^test result: ok' "$T/test.out" | awk '{s+=$4} END {print s+0}')
  if [ "$tc" -eq 0 ] && [ "$nfail" -eq 0 ]; then
    pass "cargo test --release --locked --no-fail-fast ($npass passed)"
  else
    fail "cargo test --release --locked --no-fail-fast (exit $tc, $nfail targets failed)"
    grep -E '^(test .* FAILED|failures:|    [A-Za-z_:]+$)' "$T/test.out" | show 25
  fi
else
  fail "cargo test --release --locked --no-fail-fast (no cargo)"
fi

echo "== real-binary bad-input probes"
if [ "$HAVE_PY" -eq 1 ] && [ -n "$W" ] && [ -x "$W" ]; then
  python3 - "$W" "$T/probes" <<'PY' > "$T/probes.out" 2>&1
import json
import math
import os
import shutil
import stat
import struct
import subprocess
import sys

BIN = sys.argv[1]
ROOT = sys.argv[2]
DEADLINE = 30.0
os.makedirs(ROOT, exist_ok=True)


def run_bin(args):
    try:
        p = subprocess.run(
            [BIN] + args,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=DEADLINE,
            text=True,
        )
        return p.returncode, p.stdout, p.stderr, False
    except subprocess.TimeoutExpired:
        return None, "", "TIMEOUT", True


def pack_glb(json_bytes, bin_bytes):
    jb = bytearray(json_bytes)
    while len(jb) % 4 != 0:
        jb.append(0x20)
    bb = bytearray(bin_bytes)
    while len(bb) % 4 != 0:
        bb.append(0x00)
    total = 12 + 8 + len(jb) + 8 + len(bb)
    out = bytearray()
    out += b"glTF"
    out += struct.pack("<II", 2, total)
    out += struct.pack("<II", len(jb), 0x4E4F534A)
    out += jb
    out += struct.pack("<II", len(bb), 0x004E4942)
    out += bb
    return bytes(out)


def make_minimal_glb(
    node0_t=(0.0, 0.0, 0.0),
    node1_t=(0.0, 1.0, 0.0),
    weights=None,
    indices=(0, 1, 2),
    skins_override=None,
    allow_nan_json=False,
):
    if weights is None:
        weights = [(1.0, 0.0, 0.0, 0.0)] * 3

    bin_buf = bytearray()

    def add_view(data, target=None):
        while len(bin_buf) % 4 != 0:
            bin_buf.append(0)
        off = len(bin_buf)
        bin_buf.extend(data)
        bv = {"buffer": 0, "byteOffset": off, "byteLength": len(data)}
        if target is not None:
            bv["target"] = target
        return bv

    # 2 identity MAT4 inverse bind matrices
    ident = [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
    ibm_bytes = struct.pack("<32f", *(ident + ident))
    pos_bytes = struct.pack("<9f", 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0)
    jnt_bytes = struct.pack("<12B", *([0] * 12))
    wgt_flat = [x for row in weights for x in row]
    wgt_bytes = struct.pack("<12f", *wgt_flat)
    idx_bytes = struct.pack(f"<{len(indices)}H", *indices) if indices else struct.pack("<3H", 0, 0, 0)

    bvs = [
        add_view(ibm_bytes),
        add_view(pos_bytes, 34962),
        add_view(jnt_bytes, 34962),
        add_view(wgt_bytes, 34962),
        add_view(idx_bytes, 34963),
    ]
    accessors = [
        {"bufferView": 0, "componentType": 5126, "count": 2, "type": "MAT4"},
        {
            "bufferView": 1,
            "componentType": 5126,
            "count": 3,
            "type": "VEC3",
            "min": [0.0, 0.0, 0.0],
            "max": [1.0, 1.0, 0.0],
        },
        {"bufferView": 2, "componentType": 5121, "count": 3, "type": "VEC4"},
        {"bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC4"},
        {"bufferView": 4, "componentType": 5123, "count": len(indices), "type": "SCALAR"},
    ]
    skins = (
        skins_override
        if skins_override is not None
        else [{"joints": [0, 1], "inverseBindMatrices": 0}]
    )
    doc = {
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0, 2]}],
        "nodes": [
            {"name": "root", "translation": list(node0_t), "children": [1]},
            {"name": "tip", "translation": list(node1_t)},
            {"name": "MeshNode", "mesh": 0, "skin": 0},
        ],
        "skins": skins,
        "meshes": [
            {
                "name": "Tri",
                "primitives": [
                    {
                        "attributes": {"POSITION": 1, "JOINTS_0": 2, "WEIGHTS_0": 3},
                        "indices": 4,
                        "mode": 4,
                    }
                ],
            }
        ],
        "buffers": [{"byteLength": len(bin_buf)}],
        "bufferViews": bvs,
        "accessors": accessors,
    }
    js = json.dumps(doc, allow_nan=allow_nan_json).encode("utf-8")
    return pack_glb(js, bytes(bin_buf))


def mutate_weight(src_bytes, float_index, value):
    b = bytearray(src_bytes)
    jl = struct.unpack_from("<I", b, 12)[0]
    j = json.loads(b[20 : 20 + jl].decode("utf-8"))
    bin_off = 20 + jl + 8
    acc_i = j["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_0"]
    acc = j["accessors"][acc_i]
    bv = j["bufferViews"][acc["bufferView"]]
    at = bin_off + bv.get("byteOffset", 0) + acc.get("byteOffset", 0) + float_index * 4
    b[at : at + 4] = struct.pack("<f", value)
    return bytes(b)


def write_file(name, data):
    p = os.path.join(ROOT, name)
    with open(p, "wb") as f:
        f.write(data)
    return p


def check_bad_stderr(stderr):
    if "panicked" in stderr.lower() or "stack backtrace" in stderr.lower():
        return "panicked"
    lines = [l for l in stderr.splitlines() if l.strip()]
    if len(lines) != 1:
        return f"expected 1 plain stderr line, got {len(lines)}: {stderr!r}"
    return None


def verify_reject(label, glb_path, check_codes=(2,), fix_codes=(2,)):
    errs = []
    rc, so, se, hung = run_bin(["check", glb_path])
    if hung:
        errs.append("check hung")
    elif rc not in check_codes:
        errs.append(f"check exit {rc}, want {check_codes}; stderr={se!r}")
    else:
        e = check_bad_stderr(se)
        if e:
            errs.append(f"check {e}")

    out_dir = os.path.join(ROOT, f"out_{label}")
    shutil.rmtree(out_dir, ignore_errors=True)
    os.makedirs(out_dir, exist_ok=True)
    out_glb = os.path.join(out_dir, "fixed.glb")
    rc, so, se, hung = run_bin(["fix", glb_path, "--out", out_glb])
    if hung:
        errs.append("fix hung")
    elif rc not in fix_codes:
        errs.append(f"fix exit {rc}, want {fix_codes}; stderr={se!r}")
    else:
        e = check_bad_stderr(se)
        if e:
            errs.append(f"fix {e}")
    leftover = os.listdir(out_dir)
    if leftover:
        errs.append(f"fix left half-written files: {leftover}")

    if errs:
        print(f"FAIL  probe::{label}")
        for m in errs:
            print(f"      {m}")
    else:
        print(f"PASS  probe::{label}")


# Build fixtures (clean & bleed mannequins) for real-asset probes
fix_dir = os.path.join(ROOT, "fixtures")
os.makedirs(fix_dir, exist_ok=True)
rc_c, _, se_c, _ = run_bin(["fixture", "clean", "--out", fix_dir])
rc_b, _, se_b, _ = run_bin(["fixture", "bleed", "--out", fix_dir])
clean_path = os.path.join(fix_dir, "mannequin_clean.glb")
bleed_path = os.path.join(fix_dir, "mannequin_bleed.glb")
clean_bytes = open(clean_path, "rb").read() if rc_c == 0 and os.path.isfile(clean_path) else b""

# 0. Control: minimal valid GLB must pass check cleanly (exit 0)
valid_bytes = make_minimal_glb()
valid_path = write_file("valid_minimal.glb", valid_bytes)
rc, so, se, hung = run_bin(["check", valid_path])
if not hung and rc == 0 and not se.strip():
    print("PASS  probe::control_minimal_valid_glb")
else:
    print("FAIL  probe::control_minimal_valid_glb")
    print(f"      exit={rc} hung={hung} stderr={se!r}")

# 1. Node translation: literal JSON NaN & Infinity
verify_reject(
    "node_translation_json_nan",
    write_file("t_json_nan.glb", make_minimal_glb(node1_t=(float("nan"), 0.0, 0.0), allow_nan_json=True)),
)
verify_reject(
    "node_translation_json_inf",
    write_file("t_json_inf.glb", make_minimal_glb(node1_t=(float("inf"), 0.0, 0.0), allow_nan_json=True)),
)

# 2. Node translation: non-numeric JSON ("NaN" / null)
verify_reject(
    "node_translation_non_numeric",
    write_file("t_str_nan.glb", make_minimal_glb(node1_t=("NaN", 0.0, 0.0))),
)

# 3. Node translation: child joint at 1e200 overflowing bone length / tail to NaN/Inf
verify_reject(
    "node_translation_overflow_tail",
    write_file("t_overflow_tail.glb", make_minimal_glb(node1_t=(1e200, 0.0, 0.0))),
)

# 4. Node translation: chained 1e308 overflowing rest_global to Inf
verify_reject(
    "node_translation_overflow_global",
    write_file(
        "t_overflow_global.glb",
        make_minimal_glb(node0_t=(1e308, 0.0, 0.0), node1_t=(1e308, 0.0, 0.0)),
    ),
)

# 5. Non-finite skin weights: NaN in secondary slot alongside valid 1.0
verify_reject(
    "weight_nan_secondary_slot",
    write_file(
        "w_nan_slot1.glb",
        make_minimal_glb(
            weights=[
                (1.0, float("nan"), 0.0, 0.0),
                (1.0, 0.0, 0.0, 0.0),
                (1.0, 0.0, 0.0, 0.0),
            ]
        ),
    ),
    check_codes=(1, 2),
    fix_codes=(2,),
)

# 6. Non-finite skin weights: +Inf / -Inf in secondary slot alongside valid 1.0
verify_reject(
    "weight_inf_secondary_slot",
    write_file(
        "w_inf_slot1.glb",
        make_minimal_glb(
            weights=[
                (1.0, float("inf"), 0.0, 0.0),
                (1.0, float("-inf"), 0.0, 0.0),
                (1.0, 0.0, 0.0, 0.0),
            ]
        ),
    ),
    check_codes=(1, 2),
    fix_codes=(2,),
)

# 7. Non-finite skin weight: +Inf in primary slot on mannequin_clean
if clean_bytes:
    verify_reject(
        "weight_inf_primary_slot_mannequin",
        write_file("w_inf_mannequin.glb", mutate_weight(clean_bytes, 40, float("inf"))),
        check_codes=(1, 2),
        fix_codes=(2,),
    )
else:
    print("FAIL  probe::weight_inf_primary_slot_mannequin")
    print(f"      could not build clean fixture: {se_c!r}")

# 8. Truncated GLBs (empty, short header, truncated JSON, truncated BIN)
trunc_cases = [
    ("empty", b""),
    ("short_hdr", valid_bytes[:8]),
    ("trunc_json", valid_bytes[:40]),
    ("trunc_bin", clean_bytes[: len(clean_bytes) // 2] if clean_bytes else valid_bytes[:-16]),
]
trunc_errs = []
for name, payload in trunc_cases:
    p = write_file(f"trunc_{name}.glb", payload)
    for cmd_kind in ("check", "fix"):
        out_dir = os.path.join(ROOT, f"out_trunc_{name}")
        shutil.rmtree(out_dir, ignore_errors=True)
        os.makedirs(out_dir, exist_ok=True)
        args = ["check", p] if cmd_kind == "check" else ["fix", p, "--out", os.path.join(out_dir, "out.glb")]
        rc, so, se, hung = run_bin(args)
        if hung:
            trunc_errs.append(f"{name}/{cmd_kind}: hung")
        elif rc != 2:
            trunc_errs.append(f"{name}/{cmd_kind}: exit {rc}, want 2; stderr={se!r}")
        else:
            e = check_bad_stderr(se)
            if e:
                trunc_errs.append(f"{name}/{cmd_kind}: {e}")
        if cmd_kind == "fix" and os.listdir(out_dir):
            trunc_errs.append(f"{name}/fix left files: {os.listdir(out_dir)}")
if trunc_errs:
    print("FAIL  probe::truncated_glbs")
    for m in trunc_errs:
        print(f"      {m}")
else:
    print("PASS  probe::truncated_glbs")

# 9. Zero joints (empty skins array and skin with empty joints)
zj_errs = []
for name, skins in [("skins_empty", []), ("skin_zero_joints", [{"joints": []}])]:
    p = write_file(f"{name}.glb", make_minimal_glb(skins_override=skins))
    for cmd_kind in ("check", "fix"):
        out_dir = os.path.join(ROOT, f"out_{name}")
        shutil.rmtree(out_dir, ignore_errors=True)
        os.makedirs(out_dir, exist_ok=True)
        args = ["check", p] if cmd_kind == "check" else ["fix", p, "--out", os.path.join(out_dir, "out.glb")]
        rc, so, se, hung = run_bin(args)
        if hung or rc != 2:
            zj_errs.append(f"{name}/{cmd_kind}: exit={rc} hung={hung} stderr={se!r}")
        else:
            e = check_bad_stderr(se)
            if e:
                zj_errs.append(f"{name}/{cmd_kind}: {e}")
        if cmd_kind == "fix" and os.listdir(out_dir):
            zj_errs.append(f"{name}/fix left files: {os.listdir(out_dir)}")
if zj_errs:
    print("FAIL  probe::zero_joints")
    for m in zj_errs:
        print(f"      {m}")
else:
    print("PASS  probe::zero_joints")

# 10. Zero triangles: empty indices (count=0)
verify_reject(
    "zero_triangles_empty_indices",
    write_file("zero_tris_empty.glb", make_minimal_glb(indices=())),
)

# 11. Zero triangles: all degenerate triangles ([0, 0, 1])
verify_reject(
    "zero_triangles_degenerate",
    write_file("zero_tris_degen.glb", make_minimal_glb(indices=(0, 0, 1))),
)

# 12. Unwritable output folder (file-as-parent everywhere, plus chmod 0555 dir when non-root)
blocker = write_file("not_a_dir", b"blocker")
unw_targets = [blocker]
ro_dir = None
if hasattr(os, "geteuid") and os.geteuid() != 0:
    ro_dir = os.path.join(ROOT, "readonly_dir")
    os.makedirs(ro_dir, exist_ok=True)
    os.chmod(ro_dir, stat.S_IRUSR | stat.S_IXUSR | stat.S_IRGRP | stat.S_IXGRP | stat.S_IROTH | stat.S_IXOTH)
    unw_targets.append(ro_dir)

unw_errs = []
try:
    if rc_b != 0 or not os.path.isfile(bleed_path):
        unw_errs.append(f"could not build bleed fixture: {se_b!r}")
    else:
        for base in unw_targets:
            tag = os.path.basename(base)
            cmds = [
                ("fix", ["fix", bleed_path, "--out", os.path.join(base, "fixed.glb")]),
                ("dump", ["dump", bleed_path, "--out", os.path.join(base, "w.json")]),
                ("sheet", ["sheet", bleed_path, "--out", os.path.join(base, "s.png")]),
                ("show", ["show", bleed_path, "--out", os.path.join(base, "show")]),
                ("check_vjson", ["check", bleed_path, "--vertex-json", os.path.join(base, "v.json")]),
            ]
            for cname, cargs in cmds:
                rc, so, se, hung = run_bin(cargs)
                if hung or rc != 2:
                    unw_errs.append(f"{tag}/{cname}: exit={rc} hung={hung} stderr={se!r}")
                else:
                    e = check_bad_stderr(se)
                    if e:
                        unw_errs.append(f"{tag}/{cname}: {e}")
        if ro_dir is not None and os.listdir(ro_dir):
            unw_errs.append(f"readonly_dir had files written: {os.listdir(ro_dir)}")
finally:
    if ro_dir is not None and os.path.isdir(ro_dir):
        os.chmod(ro_dir, stat.S_IRWXU)

if unw_errs:
    print("FAIL  probe::unwritable_output_folder")
    for m in unw_errs:
        print(f"      {m}")
else:
    print("PASS  probe::unwritable_output_folder")

# 13. No half-written output in weights fix when --report cannot be written
if rc_b == 0 and os.path.isfile(bleed_path):
    p_dir = os.path.join(ROOT, "fix_partial_report")
    os.makedirs(p_dir, exist_ok=True)
    out_glb = os.path.join(p_dir, "fixed.glb")
    bad_rep = os.path.join(blocker, "r.json")
    rc, so, se, hung = run_bin(["fix", bleed_path, "--out", out_glb, "--report", bad_rep])
    errs = []
    if hung or rc != 2:
        errs.append(f"exit={rc} hung={hung} stderr={se!r}")
    else:
        e = check_bad_stderr(se)
        if e:
            errs.append(e)
    leftover = os.listdir(p_dir)
    if leftover:
        errs.append(f"left half-written outputs after report failure: {leftover}")
    if errs:
        print("FAIL  probe::fix_no_half_written_on_report_fail")
        for m in errs:
            print(f"      {m}")
    else:
        print("PASS  probe::fix_no_half_written_on_report_fail")
else:
    print("FAIL  probe::fix_no_half_written_on_report_fail")
    print("      bleed fixture missing")

# 14. No half-written output in weights fix when --sheet cannot be written
if rc_b == 0 and os.path.isfile(bleed_path):
    p_dir = os.path.join(ROOT, "fix_partial_sheet")
    os.makedirs(p_dir, exist_ok=True)
    out_glb = os.path.join(p_dir, "fixed.glb")
    bad_sheet = os.path.join(blocker, "s.png")
    rc, so, se, hung = run_bin(["fix", bleed_path, "--out", out_glb, "--sheet", bad_sheet])
    errs = []
    if hung or rc != 2:
        errs.append(f"exit={rc} hung={hung} stderr={se!r}")
    else:
        e = check_bad_stderr(se)
        if e:
            errs.append(e)
    leftover = os.listdir(p_dir)
    if leftover:
        errs.append(f"left half-written outputs after sheet failure: {leftover}")
    if errs:
        print("FAIL  probe::fix_no_half_written_on_sheet_fail")
        for m in errs:
            print(f"      {m}")
    else:
        print("PASS  probe::fix_no_half_written_on_sheet_fail")
else:
    print("FAIL  probe::fix_no_half_written_on_sheet_fail")
    print("      bleed fixture missing")
PY
  py_rc=$?
  cat "$T/probes.out"
  pf=$(grep -cE '^FAIL  ' "$T/probes.out" || true)
  pp=$(grep -cE '^PASS  ' "$T/probes.out" || true)
  FAILS=$((FAILS + pf))
  if [ "$py_rc" -ne 0 ] || [ "$((pp + pf))" -eq 0 ]; then
    fail "real-binary probe runner crashed (exit $py_rc)"
  fi
else
  fail "real-binary bad-input probes (missing python3 or weights binary)"
fi

echo "== fix outputs byte-identical to main goldens"
if [ -n "$W" ] && [ -x "$W" ]; then
  WEIGHTS_BIN="$W" bash bench/fix_golden.sh --check > "$T/golden.out" 2>&1
  if tail -1 "$T/golden.out" | grep -q "^GOLDEN PASS"; then
    pass "$(tail -1 "$T/golden.out")"
  else
    fail "bench/fix_golden.sh --check"
    grep -E '^FAIL|now:' "$T/golden.out" | show 15
  fi
else
  fail "bench/fix_golden.sh --check (no weights binary)"
fi

if [ "$FAILS" -eq 0 ]; then
  echo "GATE PASS"
  exit 0
else
  echo "GATE FAIL ($FAILS)"
  exit 1
fi
