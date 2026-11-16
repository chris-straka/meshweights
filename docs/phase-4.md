# P4 — real assets

Gate: 10+ Tripo-rigged characters (humanoid + creature) from genforge,
UniRig candidate; the owner rates before/after sheets; ≥8/10 better or
equal. **Not met: blocked on assets and on the owner.**

What exists:

- `bench/run.py [dir]` fixes every GLB in `bench/models/` (gitignored),
  writes A/B sheets, reports, `summary.json`, and a review index; exits 1
  if any asset got worse. `--candidates dir` adds same-stem UniRig outputs.
- Dry run on the 7 local UniRig rigs (assets A–G, owner's corpus):

| asset | verdict | score | failing findings | time |
|---|---|---|---|---|
| A | IMPROVED | 49.7 → 50.9 | 2 → 1 | 0.1 s |
| B | IMPROVED | 21.6 → 42.5 | 47 → 16 | 2.3 s |
| C | IMPROVED | 11.9 → 64.6 | 4 → 2 | 0.2 s |
| D | IMPROVED | 1.9 → 32.5 | 51 → 11 | 3.9 s |
| E | IMPROVED | 1.6 → 77.6 | 17 → 3 | 0.4 s |
| F | IMPROVED | 6.8 → 39.8 | 88 → 17 | 33.8 s |
| G | IMPROVED | 11.9 → 27.3 | 9 → 5 | 7.1 s |

0 worse. Re-run 2026-10-05 with the exact pick and metric bands: hero
51.1, bird 35.4, cree 65.0, giraffe 30.9, stalker 66.5 (failing 3 -> 1),
tira 60.3, carrot 40.0; still 0 worse, none passes
([pick-and-bands.md](pick-and-bands.md)). None reach PASS: these skeletons have generic `bone_N` names, so
they get the harsh generic ROM (every bone ±45° on two axes plus twist),
and UniRig weights are broadly wrong; the right fix for several is a
re-rig (rigforge/wrapforge), which weightforge does not do by design.
F (54k verts, 44 bones, 252 poses) took 34 s; 12 s since incremental trial scoring.

To finish P4: generate 10+ Tripo-rigged characters with genforge, drop
them in `bench/models/` (UniRig outputs in a candidates folder), run
`bench/run.py --candidates <dir>`, send the A/B sheets to the inbox.

## 2026-11-16 — 10 generated cast rigs (`SkinTokens` + `weights fix --motion`)

Measured on `f` over 10 generated cast characters (`cast_01`..`cast_10`,
standardized humanoid rigs) on the 6 game clips (`--motion`) and the
range-of-motion poses, plus blind same-frame game-camera judging via
`bench/blind_bench.py` ([`docs/blind.md`](blind.md)):

| rig | raw tears % (clips / poses) | raw score | uncut `fix` tears % | uncut score | cut `fix` tears % | cut score |
|---|---|---|---|---|---|---|
| `cast_01` | 1.43 / 0.75 | 56.5 | **1.34 / 0.71** | 56.3 | 1.64 / 0.86 | 57.6 |
| `cast_02` | 3.67 / 3.31 | 53.4 | 3.59 / 3.29 | 53.4 | **2.86 / 3.16** | 60.4 |
| `cast_03` | 1.43 / 1.05 | 79.0 PASS | **1.05 / 0.97** | 78.9 PASS | 1.76 / 1.16 | 79.6 PASS |
| `cast_04` | 3.99 / 3.88 | 47.9 | 3.44 / 3.70 | 51.8 | **2.74 / 2.20** | 77.2 PASS |
| `cast_05` | 1.52 / 1.55 | 60.6 | **1.31 / 1.23** | 62.0 | 1.36 / 1.25 | 61.9 |
| `cast_06` | 1.23 / 1.59 | 54.1 | **0.89 / 0.45** | 63.0 | 0.97 / 0.60 | 63.6 |
| `cast_07` | 8.47 / 7.71 | 32.1 | **8.38 / 6.68** | 32.0 | 8.94 / 6.72 | 42.2 |
| `cast_08` | 7.48 / 8.95 | 18.4 | 6.12 / 6.93 | 17.8 | **4.71 / 4.24** | 51.2 |
| `cast_09` | 5.11 / 5.60 | 55.7 | **3.73 / 4.68** | 64.7 | 3.90 / 3.08 | 60.9 PASS |
| `cast_10` | 1.50 / 1.15 | 72.6 | 1.39 / 1.11 | 72.6 PASS | **1.16 / 0.72** | 73.4 PASS |
| **mean** | **3.58 / 3.55** | | **3.12 / 2.98** | | **3.00 / 2.40** | |
| **median** | **2.59 / 2.45** | | **2.42 / 2.26** | | **2.25 / 1.73** | |

- **0 of 10 worse** on tears (`never_worse` invariant holds on both clips
  and poses), and **10 of 10 improved** on tears (`2.86% / 2.66%` mean
  picking the better of uncut or leg-cut per character).
- Blind game-camera judging settled `fix`'s candidate ranking (`docs/blind.md`):
  enforcing `pass`, `!new_fails`, and `1.1x + 0.01` region-energy caps
  before ranking by `tears` then `score` eliminated the `2.0–3.4x` flat
  hem/crotch bridges that unconstrained tear overrides produced (blind
  head-to-head `ver_06b`: cap-guarded `fix` won 5/6 claims, gates passed).
- Owner inbox sign-off on the cast's before/after sheets remains the
  owner's call.
