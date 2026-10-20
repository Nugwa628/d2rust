# BUG-009: holding left click to move, then casting, stops the movement for good

- **Reported:** 2026-10-16, test build 0.1.1.91 (owner).
- **What happens:**
  1. Hold the left mouse button to walk or run.
  2. Cast a skill (right click).
  3. The character stops, and stays stopped even though the left button is still held. You have to click again to move.
- **Expected:** once the cast finishes, the character keeps moving toward the cursor while the left button is held.
- **Where to look:** the held-button movement repeat in the client controls. It probably gets cleared by the cast intent and isn't restarted while the button stays down.
- **Repro:** `tools/scenarios/bugs/BUG-009.scn` (with the left button held, a Fire Ball cast with the right button plays, then the walk resumes without a new click).
- **Status:** fixed on main (#145).
