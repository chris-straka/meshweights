# Blind pairs at the game camera (2026-11-15)

How a rig claim is judged now, why the old 0-100 score loses where it
disagrees, and the fixture numbers. The headline measure is unchanged:
**tears** — the share of the surface inside failing findings
(`docs/rig-score.md`). What is new is the proof beside each claim.

## The pair per claim

`weights blind a.glb b.glb --out dir/` makes one pair for every
failing headline finding of either rig (a claim: kind, part, and the
frame where it is worst, on the game's clips when both rigs can play
them, else on the range-of-motion poses):

- both rigs at **the same frame**, from **the same camera**, in the
  character's own paint, nothing marked, no rig named, no score shown;
- the camera is **the game camera**: HLH's follow camera, its lens and
  distance read from the game's own numbers (the `camera` block of
  `bevy/assets/data/metrics.ron`, `$HLH_METRICS` to point elsewhere;
  3.0 m and a 75° view on 2026-11-19), so a 1.75 m body fills about
  38% of the frame height (45° was assumed until 2026-11-19, 70%). The heading is the follow-camera heading
  that shows the claim best (the game camera orbits as the player
  turns); the framing never closes in on the fault. A fault too small
  to see at that distance is a fault the player cannot see either,
  and the pair is there to say so;
- which rig is LEFT and which is RIGHT is flipped by `--seed`
  (default 20261115; `bench/blind_bench.py` offsets `seed + idx` per
  run in a batch), and the pairs are shuffled by it too, so a run
  can be re-made exactly and the judge cannot learn the layout;
- two extra pairs keep the judge honest: a **can't-tell control**
  (the same rig on both sides — anything but "can't tell" fails the
  run) and a **planted bad case** (a copy of rig A with a mirrored
  limb's weights swapped left/right — the judge must pick the
  unplanted side, and tears must flag the planted copy, or the run
  fails).

The judged folder (`pair_NN.png`, `pair_NN_left.png`,
`pair_NN_right.png`, `pairs.json`) names no rig and holds no answers.
The key (`<dir>.key.json`, beside the folder, never in it) records
the sides, each claim's torn skin on each rig, both tears totals and
both 0-100 scores. `bench/blind_bench.py` drives runs of pairs.

## Scoring, and retiring the score

`weights blind-score dir/ --answers answers.json` (answers: pair →
left/right/cant_tell) lays each judged claim against the two numbers:

- judge and tears agree, score agrees too: nothing changes;
- **judge and tears agree against the score: that claim's score
  verdict is retired** — recorded in `verdict.json`
  (`score_retired_pairs`), never used as evidence for that claim;
- judge against tears: reported, and tears is not retired on a
  picture's say-so — it is the kept measure, so the claim is flagged
  for a human look instead;
- "can't tell": the claim is not confirmed by eye at the game
  camera. That is a result, not a failure.

The same rule now names the winner on `weights compare --zoom`
sheets and inside `weights fix`: pass (and no new failing findings or
blown region-energy caps) first, then tears; the score backs tears up
when tears tie, and where the score points the other way on a judged
sheet the headline says so ("…TEARS LESS… SCORE RETIRED"). The score
stays in `check`'s JSON and output as the older measure (genforge and
`fix`'s gate read it) — retiring is per verdict, where a blind pair
has actually been judged.

## Fixture benchmark (2026-11-15)

Three runs on the synthetic mannequins (`weights fixture all`; no
game assets — the owner's corpus is not in this repo), each judged
blind by a judge who saw only the pair sheets. Pictures and verdicts
are in the fleet job's artifacts.

| run | claims judged | judge vs tears | score vs judge | retired | control | planted |
|---|---|---|---|---|---|---|
| clean vs bleed | 3 of 4 (1 can't tell) | agrees 3/3 | agrees 3/3 | — | can't tell | picked out, 938 cm² torn planted vs 0 |
| no-elbow-falloff vs knee-wide-falloff | 0 of 3 (3 can't tell) | — | — | — | can't tell | picked out |
| bleed vs cape-on-wrong-bone | 4 of 4 | agrees 4/4 | agrees 2/4 | pairs 2 and 5 | can't tell | picked out |

What the pictures showed, in the judges' words: the bleed rig's hand
"a separated boxy block" against the clean hand following the arm;
the wrong-bone cape "sticks out stiffly sideways" / "lifted and
truncated around the torso" against the cape hanging down the back;
the planted swapped arms "elbow-kinked with an extra bent segment"
against one clean line. On the second run the knee-collapse and
forearm-stretch claims were honestly invisible at the game camera
("looks identical on both sides"): those tears stand as numbers, but
no picture at game distance confirms them, and the report says so.

The retirement case is the third run in miniature, and it is the
pro-rig pattern from `docs/rig-score.md` (hand-made rigs scoring 3
to 70 while tearing least): the cape rig scores 18.1 against the
bleed rig's 9.7, so the score ranks it better, while it tears 6.97%
of its surface against 1.43%. On the cape's own claims the judge
picked the bleed rig's side both times, with tears — so the score's
verdict on those claims is retired. On the bleed claim itself all
three agreed.

## P4 generated cast benchmark (2026-11-16)

Ten generated humanoid cast rigs (`cast_01`..`cast_10`, raw vs Round-3
`weights fix --motion` on the six game clips) plus three candidate-ranking
runs (`rank_01`..`rank_03`: gate-passing score pick vs unconstrained
tears override), driven by `bench/blind_bench.py --motion` on `f` with a
fresh blind judge per run shown only `pair_01.png`..`pair_08.png` (answer
keys held off the judging host until all 104 answers were locked).

| run | A → B tears % (clips) | A → B score | claims judged | judge vs tears | score vs judge | retired | gates |
|---|---|---|---|---|---|---|---|
| `cast_01` | 1.43 → 1.22 | 56.5 → 55.0 | — | — | — | — | FAIL (control) |
| `cast_02` | 3.67 → 3.41 | 53.4 → 41.2 | 2 of 6 | 1/2 | 2/2 | — | PASS |
| `cast_03` | 1.43 → 0.82 | 79.0P → 58.1 | 2 of 6 | 0/2 | 2/2 | — | PASS |
| `cast_04` | 3.99 → 2.71 | 47.9 → 49.2 | 0 of 6 | — | — | — | PASS |
| `cast_05` | 1.52 → 1.05 | 60.6 → 58.1 | 4 of 6 | 1/4 | 3/4 | pair 5 | PASS |
| `cast_06` | 1.23 → 0.98 | 54.1 → 63.0 | 4 of 6 | 2/4 | 2/4 | pair 3 | PASS |
| `cast_07` | 8.47 → 8.26 | 32.1 → 26.0 | — | — | — | — | FAIL (planted) |
| `cast_08` | 7.48 → 4.19 | 18.4 → 8.5 | 6 of 6 | 1/6 | 5/6 | pair 1 | PASS |
| `cast_09` | 5.11 → 3.25 | 55.7 → 36.3 | 4 of 6 | 2/4 | 4/4 | — | PASS |
| `cast_10` | 1.50 → 0.69 | 72.6 → 72.3 | 0 of 6 | — | — | — | PASS |
| `rank_01` | 2.33 → 1.05 | 66.1P → 58.1 | 1 of 6 | 0/1 | 1/1 | — | PASS |
| `rank_02` | 4.47 → 3.25 | 66.7P → 36.3 | — | — | — | — | FAIL (control) |
| `rank_03` | 1.82 → 0.69 | 72.9P → 72.3 | 0 of 6 | — | — | — | PASS |

What the cast run proved:

1. **The gates catch both failure modes on real textured characters.**
   Two judges claimed differences on identical control pairs and were
   discarded (`cast_01`, `rank_02`); on `cast_07` (a poncho fused to the
   forearms) `death@1.00s` had 227 cm² of raw flank shredding facing the
   camera that masked the 1,257 cm² swapped upper-arm tear. On `cast_08`
   the old 0-100 score tied the planted arm swap (`18.4` vs `PLANTED_A`
   within 1.0) while tears and the blind judge both flagged it.
2. **Pure tears-first without deformation caps loses at the game camera.**
   Across the 8 gate-passing cast runs (22 visible claims, 26 `cant_tell`
   at game distance), judge and tears retired the score on 3 claims
   (`cast_05` sleeve, `cast_06` hip satchel/arm, `cast_08` shin drag),
   but on 14 claims the blind judges backed the 0-100 score against
   `tears` (`judge_agrees_tears = 7/22`, `score_agrees_judge = 18/22`).
   Why: Round 3's `headline_choice` overrode `pick` by `tears` alone,
   ignoring `pass`, `new_fails`, and `pick`'s `1.1x + 0.01` region-energy
   cap. On uncut fused robes, tunics, and crotches (`cast_02`, `cast_03`,
   `cast_05`, `cast_08`, `cast_09`), `own-bones`, `geodesic-band`, and
   `mirror` cleared `wrong_bone` (`>6 cm`) by stretching cloth `2.0–3.4x`
   into flat bridges between the legs — below the `3.5x` hard tear line
   so `tears` fell, while region deformation energy jumped 1.5–3.7x and
   blind judges called out the flat hem/crotch bridges directly.
3. **Settling `fix`'s candidate ranking (`fix.rs`).** `fix` now enforces
   the `compare --zoom` order across both `consider` and
   `headline_choice`: never lose `pass`, never add a failing deformation
   finding, and never exceed the `1.1x + 0.01` region-energy cap; inside
   those rules, rank by `tears` (beyond `0.05%` per group), then `score`,
   with smaller weight edits (`EDIT_MU = 0.04`, `STAY_NEAR_CM2 = 15`)
   preferred where tears allow and `mirror` / `crotch-to-pelvis` reserved
   for symmetric rest poses in `headline_choice`.
   - Head-to-head blind verification on the robed character (`ver_06b`,
     uncapped `fix` vs cap-guarded `fix`): gates passed (`control`
     `cant_tell`, `planted` picked out) and the blind judge picked the
     cap-guarded `fix` on 5 of 6 claims; `cast_03` and `cast_05`
     re-verified blind against `raw` (`ver_02`, `ver_03`) passed gates
     with 0 visible regressions (`cast_03` keeps `78.9 PASS`, tears
     `1.43% → 1.05%`).
   - Cast 10 (`--motion` clips / poses): raw `3.58% / 3.55%` mean →
     uncut `fix` `3.12% / 2.98%`, cut `fix` `3.00% / 2.40%` (`2.86% /
     2.66%` best of uncut/cut per character; 0 worse on tears, 0 score
     collapses).
   - Closeness to pro rigs (`SkinTokens` → `fix` on the 5 public humanoid
     shelf rigs): `CesiumMan` skin `>2 cm off` improved `16.52% → 14.51%`
     (`p95` `1.97 → 1.80 cm`, tears `1.47% → 0.52%`), `MPFB` changed
     vertices dropped `545 → 191`, `Quaternius UBC male` `p95` `4.00 →
     3.45 cm` (tears `0.20% → 0.00%`).

## Frozen held-out set for ranking claims

`bench/held_out.json` freezes a held-out set of 2 humans (`human_01`,
`human_02`) and 3 creatures (`creature_01`..`creature_03`) by `id`,
`kind`, and `sha256` (actual GLB files remain outside the repo).
`bench/blind_bench.py` enforces the held-out set whenever
`--ranking-claim` is set:

- `--verify-manifest DIR [--held-out-manifest PATH]` checks that every
  frozen entry in the manifest matches `<DIR>/<id>.glb` (or a `.glb` in
  `DIR`) by exact `sha256`.
- `--out DIR NAME=a.glb,b.glb ... [--ranking-claim]` always hashes each
  pair's inputs into `<DIR>/run_manifest.json`; with `--ranking-claim`
  it refuses before running `weights` unless every frozen `sha256`
  appears as input `A`.
- `--human-motion PATH` and `--creature-motion PATH` route `--motion` by
  `held_out_kind` during pair generation: humans receive `--human-motion`
  (when set), creatures receive `--creature-motion` (when set, or omit
  `--motion` to play their own clips when only `--human-motion` is set),
  and any other case falls back to `--motion PATH` if given.
- `--score-batch DIR [--ranking-claim]` reads `run_manifest.json`, each
  `<name>.key.json`, and `<name>/verdict.json`; with `--ranking-claim`
  it refuses unless every frozen held-out rig is covered, all blind
  validity gates (`control_ok`, `planted_ok`, `planted_tears_ok`) pass,
  zero held-out rigs worsen on tears, and a strict majority improve on
  tears.

## Remaining limits

- A claim is drawn at its worst frame as a still. A fault that only
  shows in motion (a lagging cloth) can read as can't-tell in a
  still; `weights probe`'s unmarked loops remain the motion check.
- Fused robes, ponchos, and crotches cannot be separated by weights
  alone: cap-guarded `fix` avoids turning them into flat stretched
  sheets, while `meshkit`'s cut (`cut_fix` above: `7.48% → 4.71%` on
  `cast_08`, `3.99% → 2.74%` and `PASS` on `cast_04`) separates the
  geometry where appropriate.
