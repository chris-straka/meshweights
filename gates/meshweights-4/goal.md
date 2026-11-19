# meshweights-4: no panics, no hangs, no silent writes

Repo: meshweights (`~/Games/_blender/meshweights`). One goal, this repo only. Rust only.

## Why

`weights` reads numbers out of generated GLBs. A NaN in the file today makes it run for ever
or panic instead of saying what is wrong, and three result files are written with `let _ =`,
so they can go missing while the command exits 0. Measured on main (2026-11-19):

- one NaN in a skin's inverse bind matrix: `weights check` runs for ever (Mac and Linux);
- one NaN vertex position: runs for ever on a Mac; on Linux it panics at
  `core/src/metrics.rs:207` (`called Option::unwrap() on a None value`);
- `cli/src/main.rs:482` (`show`'s findings.json), `:572` (`before-after`'s before_after.json)
  and `:1012` (`blind-score`'s verdict.json) swallow write errors.

The tests that say what "done" means are already written: `rust/cli/tests/robust_input.rs`
(read it first). Five of its eight fail today; all eight must pass.

## What to build

1. **Non-finite input is an input error.** When a GLB is loaded (`wf::scene::Model::load` /
   `Model::from_glb`, `core/src/scene.rs`), check every vertex position and every inverse bind
   matrix value the model uses, and node transforms if they can carry one; on the first one
   that is not finite return an `Err` that names what and where, for example
   `"mesh 'Body' vertex 10: position is not finite (NaN)"` or
   `"skin 0 joint 3 'DEF-forearm.L': inverse bind matrix is not finite (NaN)"`. The CLI already
   turns a load error into `weights: <file>: <message>` and exit 2 (`fn fail`). Every command
   that loads a GLB gets this for free. Don't reject NaN skin weights: a broken weight is what
   `check` judges (today it fails the rig with D_UNWEIGHTED, exit 1); keep that.
2. **No panicking float comparisons.** Replace every `partial_cmp(..).unwrap()` in
   `rust/core/src` and `rust/cli/src` (33 today: `anatomy.rs` 6, `fix.rs` 8, `metrics.rs` 4,
   `owner.rs` 3, `poses.rs` 1, `report.rs` 1, `scene.rs` 8, `voxel.rs` 2) with `f64::total_cmp`
   / `f32::total_cmp`, keeping any `.then(..)` tie-break as it is. Careful: `total_cmp` orders
   -0.0 before +0.0 where `partial_cmp` called them equal. If that changes an output, the golden
   check (below) will say so; then use `partial_cmp(..).unwrap_or(Ordering::Equal)` at that
   site instead. Also look at `metrics.rs:207` and any other `unwrap()` that a NaN reaches.
3. **Failed writes are errors.** `main.rs:482`, `:572` and `:1012`: on a write error, return
   `fail(format!("{}: {e}", path.display()))` (exit 2) naming the file, like the other writes
   in `main.rs` do.
4. Nothing else changes: same outputs on every good input, same exit codes, same text.

## Evidence

`bash gates/meshweights-4/gate.sh` (through fleet; it builds release, about 6 cores) prints
`GATE PASS`. It checks: only `rust/core/src/` and `rust/cli/src/` changed; no
`partial_cmp(..).unwrap()` and no `let _ = fs::write` left; no test removed or ignored;
`cargo fmt --all --check`; a release build of every target with zero warnings; the eight
`robust_input` tests by name; the whole `cargo test --release --no-fail-fast`; and
`bench/fix_golden.sh --check` (every `weights fix` output on the test mannequins byte-identical
to main's; read `bench/fix_golden.sh`).

## Out of scope; don't

- Don't edit `rust/core/tests/`, `rust/cli/tests/`, `bench/`, `tests/`, `gates/`, the goldens,
  `Cargo.toml`/`Cargo.lock` or the docs. Don't add crates.
- Don't split `fix.rs` or move code between files (that is meshweights-8, later).
- Don't change any threshold, constant or output format.
- Don't start Blender or build anything on this Mac: your `check` script runs the gate
  through fleet.
