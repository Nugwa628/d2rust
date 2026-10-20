# BUG-003: two cursors on the main menu at first launch

- **Reported:** 2026-10-16, game 0.1.1.88.
- **What happens:** when the game first opens on the main menu, the Windows cursor and the game's cursor are both visible at the same time.
- **Expected:** only the game cursor.
- **Cause:** the window opened with the system pointer visible, and the front end draws the game's cursor into its own picture, so both showed. Leaving a game also turned the pointer back on.
- **Fix:** the window starts with the pointer hidden, entering the front end hides it on every visit, and leaving a game keeps it hidden (`app/session.rs`, `hidden_cursor`). Alt-Tab and window mode changes keep it hidden.
- **Status:** reopened 2026-10-16 (still open on main after Display v2); fix in PR #118.
