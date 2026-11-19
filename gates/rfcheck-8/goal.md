# rfcheck-8: re-pin glbkit after the Bevy move

Repo: meshweights (`~/Games/_blender/meshweights`). One goal, this repo only.

## Why

meshweights reads and writes GLBs through `glbkit`, a crate in the rfcheck repo, pinned by git
rev (`rust/Cargo.toml`, workspace dependency `glbkit`, today `d3262aad...`) so the checker and
the fixer can't drift. rfcheck moved its Bevy layer to 0.20 in commit
`86abd1e89fdacba39f330b79e4a9201918eafece` (rfcheck-1). Since `d3262aa`, glbkit itself gained
the `Socket_` prefix and `Socket_Hand_R` in its rig contract (`glbkit/src/rig.rs`, +7 lines).

## What to do

1. `rust/Cargo.toml`: `glbkit = { git = "https://github.com/chris-straka/rfcheck.git", rev =
   "86abd1e89fdacba39f330b79e4a9201918eafece" }`.
2. `rust/Cargo.lock`: update only glbkit's entry (`cargo update -p glbkit` resolves it; the
   lock file is metadata, no build). No other crate's version may move.
3. If the new glbkit changes any `weights fix` output (the golden check below says so), stop
   and say which outputs changed and why in your last message; don't change code to hide it.

## Evidence

`bash gates/rfcheck-8/gate.sh` (through fleet; it builds release and fetches rfcheck from
GitHub) prints `GATE PASS`. It checks the pin in `Cargo.toml` and `Cargo.lock`, that only those
two files changed, `cargo fmt --all --check`, the whole `cargo test --release --no-fail-fast`,
and `bench/fix_golden.sh --check` (every `weights fix` output byte-identical to main's).

## Out of scope; don't

- Don't edit code, tests, `bench/`, `gates/` or docs.
- Don't start Blender or build anything on this Mac.
