# BUG-012: screen edges blur when moving (looked like the HUD side bars glitching)

- **Reported:** 2026-10-16, test build 0.1.1.91 (owner).
- **What happens:** while the character moves, the borders of the screen blur, which looks bad. At first it looked like the HUD side bars were glitching.
- **Detail (owner):** the blur is on the edges the camera moves away from, and reaches some distance into the picture. Moving bottom-left blurs the top and right edges; moving top-right blurs the bottom and left. That's where the HUD looked broken. Setup: 16:9, Fullscreen, 1920x1080.
- **Expected:** the edges stay crisp while the camera slides.
- **Where to look:** V5's smooth camera.
  - While the world layer slides by the sub-tick offset, the strip it uncovers at the trailing edges is filled by clamping or smearing the edge texels.
  - Fix: compose the world layer with a margin wider than the largest per-tick camera step, so there's real picture to slide into. Or crop the presented picture by that margin.
- **Notes:** the owner thinks this may already be fixed on main. Check current main first.
- **Status:** fixed on main (#189).
