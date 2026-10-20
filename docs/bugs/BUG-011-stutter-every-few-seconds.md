# BUG-011: the whole game stutters every 1-3 seconds

- **Reported:** 2026-10-16, test build 0.1.1.91 (owner).
- **What happens:** the whole game freezes briefly every 1-3 seconds while playing.
- **Expected:** no periodic hitches.
- **Where to look:**
  - Periodic work on the main thread: saves, room loading at the view ring (V4's wider ring loads more rooms), the settings writer, audio streaming, a GC-like cache flush, and the tick catch-up.
  - Compare the frame-time trace with room loads.
  - This may share a cause with BUG-010.
- **Status:** fixed on main (#188).
