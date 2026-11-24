# meshweights documentation

Standalone, engine-agnostic skin-weight QA and repair for rigged character and creature GLBs (`weights` CLI, MIT Rust core, and GPL Blender extension).

## Active docs

| Doc | Focus |
|---|---|
| [`integration.md`](integration.md) | Pipeline QA loop, exit codes, `check --json`, `fix` report schema, custom RON pose sets |
| [`rig-score.md`](rig-score.md) | Headline surface-tears measure, finding types, animation-clip transfer, human & non-human calibration, roadmap |
| [`blind.md`](blind.md) | Same-frame game-camera blind pair protocol (`blind` / `blind-score`), control & planted-swap gates, score retirement |
| [`pick-and-bands.md`](pick-and-bands.md) | Exact branch-and-bound region picking (`pick.rs`) and metric limb-radius repair bands |

## Non-human & quadruped calibration status & roadmap

Real animation clips already separate artist-authored non-human rigs from generated creatures, while generic range-of-motion poses and pre-fixed benchmark inputs do not yet support a passing end-to-end calibration gate (see [`rig-score.md`](rig-score.md#creature-calibration-benchcreature_calibpy)):

1. **Creature-specific ROM poses and gait clips:** Replace uniform ±45° per-bone sweeps (which over-bend complex 55–107 bone artist rigs and invert pose medians) with species-appropriate pose sets and locomotion clips (`walk`, `trot`, `lunge`, `attack`).
2. **Raw pre-`fix` creature benchmark inputs:** Run calibration against un-fixed generated creature rigs so `weights fix` produces visible game-camera claims rather than sub-pixel second-pass edits (`total_claims == 0`).
3. **Asymmetric blind planted-swap frames:** Prefer opposite-phase gait frames when picking planted left/right limb-swap controls so thick manes or symmetric attack poses cannot hide the swapped limb.

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
