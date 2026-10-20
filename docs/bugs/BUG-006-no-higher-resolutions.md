# BUG-006: no options for higher resolutions

- **Reported:** 2026-10-16, game 0.1.1.88.
- **What happens:** the resolution choices stop at the game's own frame sizes (800x600, 640x480); there's no way to pick a higher resolution such as 1920x1080 or 2560x1440.
- **Expected:** resolution choices up to the monitor's own (common sizes plus "native"). The game view stays the same 800x600 picture (owner: a fixed view is fine), scaled to the chosen resolution by the Scale mode (pixel perfect, fit, stretch, wide stretch).
- **Where:** the Video page (in game and the main-menu Options screen) and the launcher's resolution setting.
- **Status:** fixed in 0.1.1.90: window sizes up to 3840x2160 (1080p, 1440p, 2160p and 4:3 multiples) and Maximized on the Video page (V0, #79 and #98), scaled sharply by Fit or Integer (V1, #82). The launcher lets the game choose (D2RUST-Launcher #8, launcher 0.1.1.4).
