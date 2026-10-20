# BUG-002: a new character's save is refused

- **Reported:** 2026-10-16, game 0.1.1.88.
- **What happens:** starting a character made on the menu fails: `the game did not start: save ...\saves\qwer.d2s: internal code 2 (new-character flag with more data) at offset 0x24`. The game stays on the menu.
- **Cause:** the menu writes a header-only stub with the "new character" status flag. The game's first save lays the full body over that stub and kept the flag, so the next load refused the file (the flag is only valid on a bare header).
- **Fix:** the save writer clears the flag on every save with a body (`app/save.rs`, `apply_live`), with a test (#71). Saves already written with the flag are repaired when loaded: the client's loader clears the flag on exactly that case (a full, undamaged save with the flag set) and loads the body (`d2s::clear_stale_new_flag`, #117).
- **Status:** partly fixed: the writer fix is in 0.1.1.89 (#71, merged with #77); the loader repair for saves already written with the flag is in PR #117.
