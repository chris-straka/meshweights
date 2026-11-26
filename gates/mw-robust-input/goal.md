# mw-robust-input: robust input handling in `rust/core` and `weights` CLI

Repo: `meshweights`. One goal, this repo only. Rust only.

## Why

Plan item `meshweights-3` asked for `rust/core/tests/robust_input.rs`, which does not exist today, and several malformed inputs and failing-write paths still silently exit 0 or leave half-written outputs on `main`:

1. **Missing core test suite:** `rust/core/tests/robust_input.rs` does not exist (only `rust/cli/tests/robust_input.rs` exists).
2. **Zero-triangle skinned meshes silently exit 0:** `Model::from_glb` (`rust/core/src/scene.rs`) checks `if bind.is_empty()` but never checks `if tris.is_empty()`. A GLB with a valid skin and vertices whose `indices` accessor is empty (`count: 0`) or whose triangles are all degenerate (`[0, 0, 1]`) loads cleanly, and `weights check` / `weights fix` exit 0 (`PASS score 100.0/100`) and write outputs.
3. **Non-finite skin weights in secondary slots silently exit 0, and non-finite weights write bad outputs or stay silent on stderr:** in `Model::from_glb` (`rust/core/src/scene.rs`), `WEIGHTS_n` slots with `NaN` or `-Inf` fail `w > 0.0` and are silently ignored, while `+Inf` is dropped in `normalize_vw`. When another slot on the same vertex is valid (e.g. `[1.0, NaN, 0.0, 0.0]` or `[1.0, +Inf, 0.0, 0.0]`), the vertex stays weighted, `weights check` exits 0 with empty stderr, and `weights fix` exits 0 while `Model::write_weights` copies the non-finite `raw_rows` verbatim into the output GLB. Even when slot 0 is `+Inf` or `NaN` (`[+Inf, 0.0, 0.0, 0.0]`), `weights check` prints nothing to stderr and `weights fix` exits 0.
4. **Malformed or overflowing node transforms silently exit 0 or produce non-finite bone tails:** `parse_nodes` / `vec3_of` (`rust/core/src/scene.rs`) silently defaults non-numeric JSON elements (e.g. `"translation": ["NaN", 0.0, 0.0]` or `[null, 0.0, 0.0]`) to `0.0` instead of returning an error. Also, `Skeleton::build` / `Skeleton::estimate_tails` never verifies that `head`, `tail`, and `bone_len` are finite, so a child joint with `"translation": [1e200, 0.0, 0.0]` overflows `(head[c] - head[j]).len()` to `inf` and sets `tail` to `(NaN, inf, NaN)` while `Model::from_glb` returns `Ok` and `weights check` exits 0.
5. **Half-written outputs on `weights fix` write failure:** `cmd_fix` (`rust/cli/src/cmd/fix.rs`) writes the fixed GLB first and then writes `<out>.report.json` (or `--report`) and `--sheet`. When the report or sheet path cannot be written, `weights fix` exits 2 but leaves the newly written `fixed.glb` (and `fixed.report.json` when `--sheet` fails) behind on disk.
6. **Overflow safety in paint reader:** `image_bytes` (`rust/core/src/paint.rs`) computes `off + len` without `checked_add`.

## In scope

Only these files may be edited or created:

- `rust/core/src/scene.rs` (`try_parse_nodes`, `Skeleton::build`, `Model`, `Model::from_glb`, `Model::write_weights`)
- `rust/core/src/paint.rs` (`image_bytes` checked offset addition)
- `rust/core/src/glb.rs` (only if needed for container/accessor error paths)
- `rust/cli/src/cmd/check.rs` (`cmd_check`)
- `rust/cli/src/cmd/fix.rs` (`cmd_fix`)
- `rust/cli/src/cmd/data.rs` (`cmd_dump`)
- `rust/cli/src/common.rs` (shared atomic/clean write helper if helpful)
- `rust/core/tests/robust_input.rs` (new integration test file for `weightforge_core`)

Any other source file under `rust/core/src/` or `rust/cli/src/` may be touched only if directly required for a bad-input error path discovered while fixing the above.

## Design to follow

1. **Strict node transform parsing and finite bone geometry (`rust/core/src/scene.rs`):**
   - Add `pub fn try_parse_nodes(glb: &Glb) -> Result<Vec<Node>>` (keeping `pub fn parse_nodes(glb: &Glb) -> Vec<Node>` intact for API compatibility) and call `try_parse_nodes(&glb)?` from `Model::from_glb`.
   - When a node specifies `"translation"`, `"scale"`, `"rotation"`, or `"matrix"`, require an array of the exact glTF length (`3`, `3`, `4`, `16`) whose every element is a finite JSON number (`Value::as_f64().filter(|x| x.is_finite())`); otherwise return `err(...)` naming the node index, node name, and field.
   - In `Skeleton::build`, after `sk.tail = sk.estimate_tails();`, verify every `head[j]`, `tail[j]`, and `sk.bone_len(j)` is finite; on the first non-finite value return `err(...)` naming the joint index and name.
2. **Reject zero-triangle skinned meshes (`rust/core/src/scene.rs`):**
   - In `Model::from_glb`, after collecting primitives and `tris`, if `tris.is_empty()` return `err("no triangles in skinned meshes (nothing to check)")`.
3. **Non-finite skin weights (`rust/core/src/scene.rs`, `rust/cli/src/cmd/{check,fix,data}.rs`):**
   - Preserve the existing protected CLI test `rust/cli/tests/robust_input.rs::nan_weight_fails_the_rig`, which asserts that `weights check` on a GLB with a `NaN` weight in slot 0 exits **1** (a failing rig with `D_UNWEIGHTED`), not exit 2.
   - Add `pub bad_weights: usize` to `Model` (count of welded vertices whose raw `WEIGHTS_n` accessor carried any non-finite value — `NaN`, `+Inf`, or `-Inf` — in any of its 4 slots).
   - In `Model::from_glb`, track `raw_bad_weight[v]` whenever any component `w` of `WEIGHTS_n` is `!w.is_finite()`, propagate `bad_weight[id] |= raw_bad_weight[v]` per welded vertex, and clear `local_w[i]` for any welded vertex with `bad_weight[i]` so it is always flagged `D_UNWEIGHTED` even when another slot held `1.0`.
   - In `Model::write_weights`, require `raw[ri].1.iter().all(|x| x.is_finite())` inside `valid` so non-finite raw weight rows are never copied verbatim into an output GLB.
   - In `cmd_check`, when `model.bad_weights > 0`, print one plain line on stderr (e.g. `eprintln!("weights: {input}: {} vertex(es) have non-finite skin weights", model.bad_weights);`) and continue reporting `D_UNWEIGHTED` with exit 1.
   - In `cmd_fix` (for input, `--source`, and every `--candidate`) and `cmd_dump`, when `bad_weights > 0`, return `fail(...)` (exit 2, one plain line on stderr, writing no files).
4. **No half-written outputs on write failure (`rust/cli/src/cmd/fix.rs`, `rust/cli/src/common.rs`):**
   - In `cmd_fix`, prepare `glb_bytes`, the serialized report JSON, and (when `--sheet` is passed) the encoded PNG bytes before committing files, and remove any file created by the run if a later write (`report_path` or `sheet_path`) fails (and avoid leaving partial or temp files behind). Return `fail(e)` (exit 2) with one plain message on stderr naming the failing path.
   - Keep `cmd_show`'s sheet-then-findings order intact so protected test `rust/cli/tests/robust_input.rs::show_fails_when_findings_json_cant_be_written` continues to pass.
   - In `rust/core/src/paint.rs::image_bytes`, use `off.checked_add(len)` before slicing `model.glb.bin`.
5. **Core robust-input test suite (`rust/core/tests/robust_input.rs`):**
   - Write at least 6 focused `#[test]` functions covering `Glb::parse` and `Model::from_glb` (and `Model::write_weights`) on small generated GLBs:
     - truncated GLBs (empty, short header, truncated JSON chunk, truncated BIN chunk),
     - non-finite / malformed / overflowing node translations and transforms (literal JSON `NaN`/`Infinity`, non-numeric JSON `["NaN", 0, 0]`, chained `[1e308, 0, 0]` overflow, child joint `[1e200, 0, 0]` bone-tail overflow),
     - non-finite vertex positions and inverse bind matrices (`NaN`, `+Inf`, `-Inf`),
     - non-finite skin weights (`NaN`, `+Inf`, `-Inf` in both slot 0 and secondary slot 1 alongside `1.0`, verifying `bad_weights > 0` and unweighted welded vertex),
     - zero joints (`"skins": []` and `"skins": [{"joints": []}]`),
     - zero triangles (empty `indices` `count: 0`, short `indices` `count: 2`, and all-degenerate triangle `[0, 0, 1]`).
6. **Zero behaviour drift on good inputs:**
   - Every `weights fix` output on the test mannequins must stay byte-identical to the goldens recorded from `main` in `bench/fix_golden/<os>-<arch>.txt` (and `bench/fix_golden/all.txt`), checked by `bench/fix_golden.sh --check`.

## Tests that must pass

- New: `cargo test --release --locked -p weightforge_core --test robust_input` (at least 6 tests passing).
- Existing CLI robust-input tests (`rust/cli/tests/robust_input.rs`, all 9 by name):
  - `nan_position_is_an_input_error`
  - `nan_bind_matrix_is_an_input_error`
  - `nan_weight_fails_the_rig`
  - `fix_refuses_nan_and_writes_nothing`
  - `clean_inputs_still_work`
  - `show_fails_when_findings_json_cant_be_written`
  - `blind_score_fails_when_verdict_json_cant_be_written`
  - `fix_and_dump_report_unwritable_outputs`
  - `malformed_empty_and_invalid_threshold_errors_exit_2`
- Full workspace suite: `cargo test --release --locked --no-fail-fast` (zero failures, no tests removed or ignored).
- Formatting and lints: `cargo fmt --all --check` and `cargo clippy --all-targets --locked -- -D warnings`.
- Golden parity: `bash bench/fix_golden.sh --check` (`GOLDEN PASS` against `bench/fix_golden/`).
- Real-binary black-box probes built into `gates/mw-robust-input/gate.sh` (all 16 probes, including the minimal valid GLB control).

## Evidence

Running `bash gates/mw-robust-input/gate.sh` from the repo root (through fleet on a clean checkout) prints `GATE PASS` and exits 0.

## Out of scope; do not edit

- Do **not** edit `gates/mw-robust-input/` (the gate or any of its inputs), `bench/` (including `bench/fix_golden.sh` and `bench/fix_golden/*.txt`), `tests/` (goldens), `rust/cli/tests/` (including `rust/cli/tests/robust_input.rs` and `rust/cli/tests/cli_contract.rs`), or any pre-existing test file in `rust/core/tests/`.
- Do **not** edit `rust/Cargo.toml`, `rust/Cargo.lock`, `rust/core/Cargo.toml`, `rust/cli/Cargo.toml`, `poses/`, `blender/`, `docs/`, `PLAN.md`, `README.md`, or `AGENTS.md`. Do not add crates.
- Do **not** change any deformation threshold, solver constant, or output format on valid inputs.
- Do **not** start Blender or run heavy builds on the owner's Mac by hand; run the gate through fleet.
