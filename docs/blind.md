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
- the camera is **the game camera**: HLH's follow camera
  (`bevy/src/fx.rs` `follow_camera`, 3.0 m behind the character at the
  chest through the default 45° lens), so a 1.75 m body fills about
  70% of the frame height. The heading is the follow-camera heading
  that shows the claim best (the game camera orbits as the player
  turns); the framing never closes in on the fault. A fault too small
  to see at that distance is a fault the player cannot see either,
  and the pair is there to say so;
- which rig is LEFT and which is RIGHT is flipped by `--seed`
  (default 20261115), and the pairs are shuffled by it too, so a run
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
sheets: pass first, then tears; the score only backs tears up, and
where it points the other way the headline says so
("…TEARS LESS… SCORE RETIRED"). The score stays in
`check`'s JSON and output as the older measure (genforge and `fix`'s
gate read it), and `fix` still ranks candidates as before — retiring
is per verdict, where a blind pair has actually been judged.

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

## Limits and what's next

- Fixtures are grey mannequins; their paint is flat, so this proves
  the protocol and the geometry, not the look on a textured hero.
  The same commands take any two rigs and `--motion hero.glb` for
  the game's clips; rerunning on the owner's cast (P4's set) is the
  next step, through `bench/blind_bench.py`, with the judge shown
  only the pair folders.
- A claim is drawn at its worst frame as a still. A fault that only
  shows in motion (a lagging cloth) can read as can't-tell in a
  still; `weights probe`'s unmarked loops remain the motion check.
- `fix` still picks by gate-then-score internally (with the tears
  guard on top). Whether its candidate ranking should switch to
  tears-first the way the zoomed compare now does is a P4 question
  for the cast run, not settled here.
