# P5 — Blender extension and pipeline adapter hook (archive)

Gate: end-to-end weight-gate flow: auto-rigged mesh fails check → fixed →
passes, with before/after sheets. An external pipeline adapter adds
`check|fix` (`check --json`; `fix --out --sheet --report`; exit 1 keeps
the best-so-far output and routes to re-rigging). Its adapter test runs
this repo's binary on the bleed fixture: check fails → fix 9.7 → 89.6 →
recheck passes (5 tests: green). Rehearsal run (2026-10-05, SkinTokens
rig): the weights gate passes on the humanoid rehearsal mesh (62.7 ->
76.0); the chain then stops after animate, where an extreme `attack_3`
overhead swing fails recheck ([../pick-and-bands.md](../pick-and-bands.md)).

- `blender/weightforge/` (GPL, subprocess only): Check Weights (paints
  the `weightforge_bad` color attribute), Pose Sheet (image datablock),
  Fix Weights (writes bone vertex groups, one undo step, loads the A/B
  sheet). Results map back by exact vertex position.
- Headless test `blender/tests/test_headless.py` (Blender 5.2): imports
  the bleed fixture through Blender's glTF importer, check finds the bleed
  and paints 177 verts, fix gives FIXED 9.7 → 89.6 with 0 hand verts left
  on the thigh and 0 vertices moved, the re-exported rig passes the CLI
  check, the sheet renders.

Typical external pipeline weight gate: run `weights check in.glb --json`;
on exit 1 run `weights fix in.glb --out fixed.glb --sheet ab.png [--candidate other.glb]`,
then `weights check fixed.glb`; exit 0 means pass; keep `ab.png` +
`fixed.report.json` for review or blind judging.
