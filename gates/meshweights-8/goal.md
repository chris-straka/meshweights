# meshweights-8: split fix.rs, no behaviour change

Repo: meshweights (`~/Games/_blender/meshweights`). One goal, this repo only. Rust only.

## Why

`rust/core/src/fix.rs` is about 1,970 lines: shared helpers, the weight candidates, the
picking between them, and the fix itself in one file. Long files hide bugs. Split it by part,
moving code, not changing it: every `weights fix` output must stay byte for byte the same.

## What to build

`rust/core/src/fix.rs` becomes the module folder `rust/core/src/fix/` (lib.rs's `mod fix;`
stays as it is):

- `fix/helpers.rs`: the section headed `// --- helpers` (today from `pub fn smooth` to
  `fn deform_bones`, including `pub mod widths`);
- `fix/candidates.rs`: the section headed `// --- candidates` (`cleanup` to `external`);
- `fix/picking.rs`: the section headed `// --- picking` (`l1` to `summary`, with `EDIT_MU`);
- `fix/mod.rs`: the types at the top (`Method` .. `FixResult`), `fix`, `fix_once`,
  `ranks_below`, `weight_error`, the module declarations, and `pub use` lines so every `pub`
  item fix.rs had is still `weightforge_core::fix::<name>`. At most 1,050 lines.

Read the file in parts; line numbers move. Cut and paste: same code, same order inside each
function, same doc comments and section lines. Change only what moving needs: `use` lines,
and `pub(super)` / `pub(crate)` on items another part now calls. No renames, no reformatting
beyond `cargo fmt`, no new logic.

## Evidence

`bash gates/meshweights-8/gate.sh` (through fleet; it builds release, about 6 cores) prints
`GATE PASS`. It checks the four files and mod.rs's length, that every doc-comment and section
line of the base's fix.rs is still in `fix/`, that every `pub` item is still reachable at
`weightforge_core::fix::<name>` (a probe that names each one must compile), the scope (only
fix.rs and `fix/`), `cargo fmt --all --check`, a release build of every target with zero
warnings, the whole `cargo test --release --no-fail-fast` with no test lost, and
`bench/fix_golden.sh --check` (every `weights fix` output byte-identical to main's).

## Out of scope; don't

- Don't edit tests, `bench/`, `gates/`, the goldens, Cargo files or docs.
- Don't touch other source files; don't change any number or threshold.
- Don't start Blender or build anything on this Mac.
