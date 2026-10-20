# BUG-004: main-menu Options screen has no panel

- **Reported:** 2026-10-16, game 0.1.1.88.
- **What happens:** the Options (Settings) screen on the main menu draws its rows (text bars, sliders) with nothing behind them, floating over the menu art.
- **Expected:** the rows on a panel, like the in-game Esc options menu (its art and pentagrams).
- **Cause:** the screen drew the Options rows as plain text straight over the main menu's picture.
- **Fix:** the main menu's Display Options screen (`ui/front_end/screens/options.rs`) darkens the menu art by about three quarters, puts the page on a solid black panel framed by the game's box border, and draws the rows with the in-game menu's art (`esc_art`: label and value images, slider bars and skulls, spinning pentagrams); D2RUST's own display rows are text in the menu's colours.
- **Status:** fixed on main by #100 (Display Options on the main menu); not in a release yet (0.1.1.90 predates it).
