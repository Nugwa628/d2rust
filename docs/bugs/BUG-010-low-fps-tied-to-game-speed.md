# BUG-010: the game runs at 4-5 fps; FPS follows game speed

- **Reported:** 2026-10-16, test build 0.1.1.91 (owner).
- **What happens:**
  - In game, the picture updates at about 4-5 fps and feels laggy.
  - With the dev tools' game speed at 400%, FPS rises to about 20.
  - So FPS follows the simulation's tick rate, not the display.
- **Expected:**
  - The simulation keeps its own rate, and the picture updates at the display rate (VSync or the frame limit).
  - Real motion (V5, V6) draws the in-between frames.
- **Where to look:**
  - Why the game produces only about 5 ticks a second instead of 25. Check the tick loop's timing and catch-up, and whether one tick costs about 200 ms (CPU compose, readback, a blocking wait).
  - Whether presentation only happens on tick frames. The V5/V6 between-tick drawing may not be running, or it may be blocked by the tick work.
  - Profile one in-game second: tick time, compose time, present time.
- **Status:** fixed on main (#186).
