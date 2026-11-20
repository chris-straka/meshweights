# meshweights-2: Freeze held-out set and refuse ranking claims without a held-out run

## What exists today
`bench/held_out.json` now freezes the held-out set of 2 humans (`human_01`, `human_02`) and 3 creatures (`creature_01`, `creature_02`, `creature_03`) by `id`, `kind`, and `sha256` (actual GLB files stay outside the repo).

However, `bench/blind_bench.py` ignores `bench/held_out.json` completely and never refuses ranking claims that omit or tamper with the held-out set.

## What to build
Update `bench/blind_bench.py` (plain Python 3 stdlib only, keeping all existing CLI flags backwards-compatible) to support and enforce the frozen held-out set:

1. **Manifest handling (`--held-out-manifest PATH`)**:
   - Default path: `<repo>/bench/held_out.json`.
   - Validate the manifest has at least 2 `"human"` and 3 `"creature"` entries with valid 64-char hex `sha256` digests.
2. **Manifest verification (`--verify-manifest DIR`)**:
   - Check that for every entry in `--held-out-manifest`, `<DIR>/<id>.glb` exists (or a `.glb` in `DIR` matches its exact `sha256` — preferring `<DIR>/<id>.glb`).
   - Exit 0 if every entry matches its frozen `sha256`; exit non-zero with a clear message naming the missing/mismatched `id` and `sha256` otherwise.
3. **Pair generation (`--out DIR NAME=a.glb,b.glb ...` + optional `--ranking-claim`)**:
   - Always compute `sha256_a` and `sha256_b` for each run and write `<DIR>/run_manifest.json` recording `runs: [{"name", "sha256_a", "sha256_b", "held_out_id", "held_out_kind"}, ...]`.
   - When `--ranking-claim` is passed:
     - **Refuse** (exit non-zero with an error message mentioning `held-out`) BEFORE running `weights` unless every frozen `sha256` in `--held-out-manifest` appears as `sha256_a` (input rig A) of a run in the batch!
4. **Batch ranking-claim verification (`--score-batch DIR` + optional `--ranking-claim`)**:
   - Read `<DIR>/run_manifest.json`, each run's `<DIR>/<name>.key.json`, and `<DIR>/<name>/verdict.json`.
   - When `--ranking-claim` is passed:
     - **Refuse** (exit non-zero with a clear message mentioning `held-out`) if:
       - Any frozen `sha256` from `--held-out-manifest` is missing from `run_manifest.json` (`sha256_a`);
       - Any held-out run is missing its `.key.json` or `verdict.json`;
       - Any held-out run failed its blind validity gates (`not verdict["control_ok"]`, `not verdict["planted_ok"]`, or `not verdict["planted_tears_ok"]`);
       - Any held-out run got worse on tears (`tears_pct["B"] > tears_pct["A"] + 1e-9`);
       - Fewer than a strict majority of held-out rigs (`improved * 2 <= len(held_out)`) improved on tears (`tears_pct["B"] < tears_pct["A"] - 1e-9`).
     - Print a concise summary table and exit 0 when all checks pass.
5. Keep existing `--out DIR NAME=a,b` (without `--ranking-claim`) and `--score DIR/NAME --answers ...` working identically. Never hardcode private paths or owner asset names.

## Allowed files
Only edit:
- `bench/blind_bench.py`
- `docs/blind.md` (optional short note)

Do not edit `bench/held_out.json` or anything under `gates/`.

## Gate
`bash gates/meshweights-2/gate.sh` must exit 0 with `GATE PASS`.
