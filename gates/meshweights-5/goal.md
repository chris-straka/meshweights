# meshweights-5: Creature clips in the check

## What is broken today
1. `weightforge_core::motion_for` (`rust/core/src/lib.rs`) defaults `spec.clips` to humanoid `motion::GAME_CLIPS` (`["walk", "run", "attack_1", "dodge", "hit", "death"]`) whenever `--motion <src.glb>` is given without `--motion-clips`. When checking a creature with a creature-gait / creature-clips source (which carries clips like `walk`, `trot`, `lunge`, `attack`, `idle`), humanoid clip names are skipped and the creature's own clips are never played.
2. `weightforge_core::motion::transfer` (`rust/core/src/motion.rs`):
   - Requires at least 8 role-mapped bones (`map.len() < 8`), which rejects 2-legged `Body::Legged` creatures and `Body::Other` (winged/legless) creatures even when `--motion` is a clip-bearing copy of the same skeleton (as `creature-gait` / `creature-clips` outputs).
   - Its `limb` rest-alignment predicate only lists humanoid roles, missing `leg*` (`leg0_upper`, `leg0_lower`, ...) and `tail*`.
3. `weightforge_core::blind::planted_bad` (`rust/core/src/blind.rs`) only tries the six hardcoded role prefixes `["upper_arm", "forearm", "thigh", "shin", "hand", "foot"]`, so it returns `None` on `Body::Legged` creatures whose mirrored leg roles are `leg0_upper.L/.R`, `leg0_lower.L/.R`, etc.

## What to build
1. In `rust/core/src/lib.rs` (`motion_for`) and `rust/core/src/motion.rs`:
   - When `spec.clips` is `None` and the model or motion source is not `Body::Humanoid`, play every clip present in the motion source (in clip order) instead of asking for humanoid `GAME_CLIPS`. Keep `GAME_CLIPS` unchanged for humanoids.
   - In `motion::transfer`, support transferring clips on same-skeleton `--motion` sources (matching joint names and parent links, so `Body::Other`, `Body::Legged`, and `Body::Quadruped` rigs can always be checked against their own `.gait.glb` / `.moving.glb` clips without losing bones).
   - For cross-rig role transfer, include `"leg"` and `"tail"` in the limb rest-alignment predicate and allow `Body::Legged` rigs with at least 4 mapped roles (`>= 4`; keep `>= 8` for `Humanoid` / `Quadruped`).
2. In `rust/core/src/blind.rs` (`planted_bad`):
   - Keep the existing six role prefixes first (preserving exact behavior for humanoids and quadrupeds), then also try any other mirrored `<base>.L` / `<base>.R` role pairs in `model.skel.anatomy.roles` (deterministic sorted order) so `Body::Legged` creatures get a planted left/right leg swap too.
3. Keep all humanoid behavior byte-identical (`bench/fix_golden.sh --check`), zero compiler warnings, `cargo fmt --all --check` clean, and deterministic across macOS arm64 and Linux x86_64.

## Allowed files
Only edit files under:
- `rust/core/src/lib.rs`
- `rust/core/src/motion.rs`
- `rust/core/src/blind.rs`
- `rust/core/src/poses.rs` (only if needed, e.g., helper for listing clip names)
- `rust/cli/src/main.rs` (only if needed)

Do not modify tests, `gates/`, or `bench/`.

## Gate
`bash gates/meshweights-5/gate.sh` must exit 0 with `GATE PASS`.
