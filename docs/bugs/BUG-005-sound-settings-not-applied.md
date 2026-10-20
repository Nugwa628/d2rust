# BUG-005: sound settings on the Options screen don't take effect

- **Reported:** 2026-10-16, game 0.1.1.88.
- **What happens:** changing sound settings (volume) on the main-menu Options screen does nothing: there's no Apply button and the change isn't applied automatically.
- **Expected:** the change is heard at once (applied live), as in the in-game options, and saved.
- **Cause:** the screen only saved its settings to `settings.toml` for the next game start; the menu music never read them, and the window and frame pacing weren't told.
- **Fix:** every change goes through the same path as the in-game Options page (`app::config::commit_settings`): saved at once, and the window mode and size, scale, VSync, frame limit and menu sides follow on the next frame. The menu music follows Sound and Music Volume as they move (`FrontMusic::set_levels`).
- **Status:** fixed on main by #100 (Display Options on the main menu); not in a release yet (0.1.1.90 predates it).
