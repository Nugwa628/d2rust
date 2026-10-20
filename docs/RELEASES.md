# Releases

Every game build released to players, newest first, with what it holds. The full notes and the files are on
the forge release pages (`WerdeGames/D2RUST` → Releases); the files themselves are stored in
`WerdeGames/D2RUST-builds` and linked from each release. Releases are made by hand when the owner asks for one,
built on BIGRIG from `main`. Game and launcher are versioned separately.

| Game | Launcher | Date | Built from | What it is |
|---|---|---|---|---|
| [0.1.1.90](https://git.werdegames.com/WerdeGames/D2RUST/releases/tag/v0.1.1.90) | 0.1.1.4 | 2026-10-16 | `main` 62d7a91 | Test build: Display v2 light tier plus V5-V7, V10 and window modes |
| [0.1.1.89](https://git.werdegames.com/WerdeGames/D2RUST/releases/tag/v0.1.1.89) | — | 2026-10-16 | `main` 2f68787 (#77) | Beta 0.1's game again (display and smoothness work taken out), two fixes |

## 0.1.1.90 (test build)

Files: `D2RUST-Game-0.1.1.90.7z`, `D2RUST-Setup-0.1.1.4.exe`. Launcher 0.1.1.4 is needed for 16:9 (older
launchers force 800x600).

What's in it ([Display v2](plans/game/display-v2.md) modules):
- **16:9 view (1068x600)** as the default, 4:3 (800x600, 640x480) as options (V3, V0 Resolution row).
- **The world live to the wider edges:** every monster on screen is really there (V4).
- **Sharp scaling** to 1080p, 1440p and 2160p (Fit, the default), Integer as an option (V1).
- **Window modes:** Windowed (bordered, maximized; the default), Windowed Borderless, Fullscreen; Alt+Enter
  toggles Windowed Borderless (V0).
- **Real motion:** smooth camera, units and missiles between ticks (V5, V6).
- **VSync On/Off and Frame Limit** (V2).
- **HUD at 16:9** (V7) and **main menu at 16:9** (V10).

Known issues at release: panels inside the centred 4:3 area (fixed by V8), automap and full-screen overlays
4:3 (V9), no Display Options button on the main menu (#100), HUD side-bar seams (#102), window modes not tried on
a real monitor, no Intel iGPU check yet, text not at the screen's resolution (V11).

## 0.1.1.89

File: `D2RUST-Game-0.1.1.89.7z`. Beta 0.1's game with the first display and smoothness work taken out (#77),
plus two fixes: the game no longer quits at game start on some GPUs (Intel HD 630, Vulkan), and a character made
on the menu loads again after its first save.
