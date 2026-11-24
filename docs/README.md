# meshweights documentation

Standalone, engine-agnostic skin-weight QA and repair for rigged character and creature GLBs (`weights` CLI, MIT Rust core, and GPL Blender extension).

## Active docs

| Doc | Focus |
|---|---|
| [`integration.md`](integration.md) | Pipeline QA loop, exit codes, `check --json`, `fix` report schema, custom RON pose sets |
| [`rig-score.md`](rig-score.md) | Headline surface-tears measure, finding types, animation-clip transfer, human & non-human calibration, roadmap |
| [`blind.md`](blind.md) | Same-frame game-camera blind pair protocol (`blind` / `blind-score`), control & planted-swap gates, score retirement |
| [`pick-and-bands.md`](pick-and-bands.md) | Exact branch-and-bound region picking (`pick.rs`) and metric limb-radius repair bands |

## Creature calibration & blind checks

Non-humanoid QA supports anatomy-keyed ROM presets (`poses/quadruped.ron`, `poses/winged.ron`, `poses/multi_limb.ron`), chain-distributed conservative bends for deep spine/tail/wing/fin/tentacle skeletons, optional neutral creature gait clips (`--gait`: `walk`, `trot`, `lunge`, `attack`), opposite-phase asymmetric blind planted-swap selection (`asymmetric_stride` fallback when poses are symmetric), and raw pre-`fix` calibration enforcement (`bench/creature_calib.py --require-raw`); see [`rig-score.md`](rig-score.md#creature-calibration-benchcreature_calibpy) and [`blind.md`](blind.md).

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
