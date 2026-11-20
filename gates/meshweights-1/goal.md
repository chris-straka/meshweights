# meshweights-1: Calibrate tears on creatures (`bench/creature_calib.py`)

## What to build
Write `bench/creature_calib.py` (plain Python 3, stdlib only, zero hardcoded private paths or owner asset names) to calibrate `weights`'s tears measure on hand-made artist animals against generated creatures (before and after `weights fix`) on poses and creature-gait clips, plus blind pairs at the game camera.

### Mode 1: Measure & generate blind pairs
```
python3 bench/creature_calib.py \
  --weights PATH (default: <repo>/rust/target/release/weights) \
  --artists-dir DIR (and/or repeatable --artist NAME=PATH) \
  --generated-dir DIR (and/or repeatable --generated NAME=PATH) \
  --out DIR [--seed N] [--top N] [--creature-motion PATH]
```
1. Discover artist `.glb` files (sorted `*.glb` in `--artists-dir`, or `<subdir>/truth.glb` if `<subdir>/truth.glb` exists; plus any `--artist NAME=PATH`) and generated `.glb` files (sorted `*.glb` in `--generated-dir`, plus any `--generated NAME=PATH`). Require at least 1 artist and 1 generated creature.
2. For each artist creature:
   - Run `weights check <glb> --json` (adding `--motion <creature-motion>` if given; otherwise `weights check` uses the file's own clips).
   - Parse JSON stdout; fail loudly (exit non-zero, never treat a crash/invalid JSON as `0.0` tears!).
   - Extract `poses` and `game clips` (when present) `tears_pct` from `headline`, plus `score`.
3. For each generated creature `<name>`:
   - Run `weights check <glb> --json` (before) -> fail loudly on crash/bad JSON.
   - Run `weights fix <glb> --out <OUT>/fixed/<name>_fixed.glb` (passing `--motion <creature-motion>` if given, else `--motion <glb>` when the input had `game clips`, so `fix`'s headline guard protects creature clips too).
   - Run `weights check <OUT>/fixed/<name>_fixed.glb --json` (after) -> fail loudly on crash/bad JSON.
   - Run `weights blind <glb> <OUT>/fixed/<name>_fixed.glb --out <OUT>/blind/<name> --seed <seed+idx> --top <top>` (passing `--motion <creature-motion>` if given). Keep `<OUT>/blind/<name>.key.json` beside the judged directory, never inside it.
4. Compute medians (`statistics.median`):
   - `artist_median_poses`, `generated_before_median_poses`, `generated_after_median_poses`
   - `artist_median_clips`, `generated_before_median_clips`, `generated_after_median_clips` (when clips exist)
   - `worse_count`: number of generated creatures where `after_poses > before_poses + 1e-9` or (`after_clips` and `before_clips` present and `after_clips > before_clips + 1e-9`).
5. Write `<OUT>/calib.json` and `<OUT>/calib.md` (concise markdown table + medians).
6. Exit 0 only if:
   - `artist_median_poses < generated_before_median_poses` (and when both groups have clips, `artist_median_clips < generated_before_median_clips`), AND
   - `worse_count == 0`.
   Otherwise write the files and exit 1 with a clear failure reason on stderr.

### Mode 2: Score blind answers & final calibration verdict (`--score DIR`)
```
python3 bench/creature_calib.py --score DIR [--weights PATH] [--min-agreement 0.80]
```
1. Read `<DIR>/calib.json`.
2. For every generated creature `<name>` listed in `calib.json`:
   - Look for `<DIR>/blind/<name>/verdict.json`; if missing and `<DIR>/blind/<name>/answers.json` exists, run `weights blind-score <DIR>/blind/<name> --answers <DIR>/blind/<name>/answers.json` to produce `verdict.json`.
   - Verify `verdict.json` exists and has `control_ok is True`, `planted_ok is True`, and `planted_tears_ok is True`.
   - Accumulate `total_claims += verdict["claims_judged"]` and `total_agrees += verdict["judge_agrees_tears"]`.
3. Compute `agreement_rate = total_agrees / total_claims` (when `total_claims > 0`, else `0.0`).
4. Gate passes (`pass = True`, exit 0) ONLY when ALL hold:
   - Separation holds: `artist_median_poses < generated_before_median_poses` (and `artist_median_clips < generated_before_median_clips` if both present);
   - Never-worse holds: `worse_count == 0`;
   - Every generated creature's blind run passed `control_ok`, `planted_ok`, and `planted_tears_ok`;
   - `total_claims > 0` and `agreement_rate >= min_agreement` (default `0.80`).
5. Write `<DIR>/calib_verdict.json` (`pass`, `separation_ok`, `never_worse_ok`, `blind_gates_ok`, `total_claims`, `total_agrees`, `agreement_rate`, `reasons`) and print a concise summary. Exit 0 on pass, 1 on fail.

## Allowed files
Only create/edit:
- `bench/creature_calib.py`
- `docs/rig-score.md` or `docs/blind.md` (optional short note)

Do not touch `gates/` or any Rust files.

## Gate
`bash gates/meshweights-1/gate.sh` must exit 0 with `GATE PASS`.
