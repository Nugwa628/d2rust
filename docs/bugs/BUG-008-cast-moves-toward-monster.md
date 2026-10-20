# BUG-008: casting at a monster moves or dashes the character toward it

- **Reported:** 2026-10-16, test build 0.1.1.91 (owner).
- **What happens:**
  - When you cast a ranged skill (for example Fireball) at a monster, the character moves toward the monster while casting, or jumps at it as if the skill were a dash or charge.
  - It isn't a "stop-cast disabled" problem: the skill itself seems to carry the character forward.
- **Expected:** a ranged skill cast at a monster plays the cast in place and fires the missile. The character moves only for melee skills that need range.
- **Where to look:**
  - the skill-on-target intent: right-click on a unit with a ranged skill should not turn into "move into range" or approach;
  - the server-side skill start for missile skills: no position change;
  - V6 unit motion between ticks (`unit_motion.rs`): make sure it isn't drawing a slide or extrapolation toward the target.
- **Status:** open.
