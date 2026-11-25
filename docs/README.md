# meshweights documentation

Standalone, engine-agnostic skin-weight QA and repair for rigged character and creature GLBs (`weights` CLI, MIT Rust core, and GPL Blender extension).

## Active docs

| Doc | Focus |
|---|---|
| [`integration.md`](integration.md) | Pipeline QA loop, exit codes, `check --json`, `fix` report schema, custom RON pose sets |
| [`rig-score.md`](rig-score.md) | Headline surface-tears measure, finding types, animation-clip transfer, human & non-human calibration, roadmap |
| [`blind.md`](blind.md) | Same-frame game-camera blind pair protocol (`blind` / `blind-score`), control & planted-swap gates, score retirement |
| [`pick-and-bands.md`](pick-and-bands.md) | Exact branch-and-bound region picking (`pick.rs`) and metric limb-radius repair bands |

## Creature calibration status

The creature gate (`bench/creature_calib.py --score`) is **FAIL** on its last real run (2026-11-20); the default range of motion is ±45° per weighted bone for bodies without a preset, and the blind planted case is one mirrored role pair at its worst tearing frame. Raw pre-`fix` enforcement (`--require-raw`) and opt-in gait clips (`--gait`) exist. Held-out sets score a change once and are never used to tune it. See [`rig-score.md`](rig-score.md#creature-calibration-benchcreature_calibpy) and [`blind.md`](blind.md).

## Archived R&D and gate logs (`archive/`)

Historical milestone gates and exploratory notes kept for reference:

- [`archive/phase-0.md`](archive/phase-0.md) — P0: deformation checker thresholds and synthetic fault fixtures
- [`archive/phase-1.md`](archive/phase-1.md) — P1: range-of-motion pose sheets and A/B comparison renders
- [`archive/phase-2.md`](archive/phase-2.md) — P2: weight cleanup, geodesic voxel binding, region picking, seam blending
- [`archive/phase-3.md`](archive/phase-3.md) — P3: surface transfer + harmonic inpainting and DQS optimization
- [`archive/phase-4.md`](archive/phase-4.md) — P4: early character & cast benchmark runs (superseded on ranking by [`rig-score.md`](rig-score.md) and [`blind.md`](blind.md))
- [`archive/phase-5.md`](archive/phase-5.md) — P5: Blender extension and external route adapter notes
- [`archive/research-2026-11.md`](archive/research-2026-11.md) — Literature survey and candidate ranking for fused limbs/cloth
- [`archive/review-2026-11-13.md`](archive/review-2026-11-13.md) — Rounds 1–3 code review, headline guard, and candidate tuning
- [`archive/unfuse-log.md`](archive/unfuse-log.md) — Split-crotch / fused-legs R&D experiments
