---
id: display-v2
title: Display v2 (true 16:9, sharp scaling, real motion, vsync and frame limit)
area: game
status: done
order: 2
modules: V0-V11
light: V0-V4
depends_on: []
---

# Plan: Display v2

Status: **merged through V11**; V12/V13 dropped in #267 (owner, 2026-10-17). It's built with `docs/FEATURE-METHOD.md`. It replaces the first
display update (removed from the plans on 2026-10-17): that code was reverted to beta 0.1 on 2026-10-16 (PR #77).
This plan starts from beta 0.1's display and reuses the reverted modules where they fit; their code is in git
history: D1 #47, D3 #46, D6 #48, S2 #21, S3 #27, S5 #55.

## Goal

What is fully needed (owner):
- **True 16:9.** The game renders a 16:9 picture and shows more of the world to the sides, at **1080p, 1440p and
  2160p**. 4:3 stays available as resolution options (800x600, 640x480, unchanged from today).
- **Crisp scaling on almost every system.** One single-pass fragment shader works on every GPU Bevy runs on,
  Intel iGPUs and Vulkan/DX12 included.
- **Real motion, not prediction.** Between game ticks the camera and every unit move smoothly from the simulation's
  real positions. Nothing is guessed from rendered frames.
- **VSync On/Off and a Frame Limit,** with low input latency.

Owner rules:
- **No FSR and no edge-aware upscaling filters** (xBR, ScaleFX, MMPX): the art is shown as it is, crisp.
- 16:9 is the default view; 4:3 resolutions are options. In multiplayer every player in a game gets the same view.
- No third-party shader code is adapted (`CLAUDE.md` hard rule 3).

## Decisions taken

- **16:9 frame: 1068x600.**
  - It keeps today's vertical view (600 lines), adds 268 px of width, and is within 0.1% of 16:9.
  - It scales by 1.8x at 1080p, 2.4x at 1440p and 3.6x at 2160p, filling the screen (at most 1-2 px trimmed at the
    edges).
- **Scaler: sharp bilinear (pixel antialiasing).**
  - Every game pixel stays a solid block; only the one screen pixel where two game pixels meet is blended.
  - At whole-number scales it is exactly nearest-neighbour.
  - Why this one: pure integer scaling can't cover 1080p, 1440p and 2160p with a usable 16:9 view (the only frame
    that scales by whole numbers to all three is 360 lines tall). Nearest at a fractional scale gives uneven pixel
    widths and shimmers under sub-pixel camera motion; sharp bilinear stays even and smooth.
  - It is one pass, one texture read (D6's `sharp.wgsl`).
- **Integer mode** stays an option (nearest, black bars) for 4:3 frames and for players who want exact pixels.

## Where things are today (beta 0.1)

- **Frame:** fixed at 800x600 or 640x480, chosen at startup (`rules/camera.rs` `FrameSize`). The camera,
  world-to-screen maths, culling, the draw-order grid and the compositor buffers (`gpu_compositor/`,
  `world_view/node.rs`) all assume it.
- **Presentation:** `world_view/present.rs` `present_scale`, a nearest-sampled sprite at an integer scale, centred,
  in physical pixels (divided by `window.scale_factor()`).
- **Missing:** no borderless-at-native mode, no resolution list, no vsync or frame-limit settings (Bevy defaults).
- **Drawing:** the world is composed only on tick frames (25 Hz). The cursor is drawn into the frame.
- **World range:** the world is live within `NEAR_GAP` rooms of the player. Unit visibility follows the view.
  HUD and panels are laid out for 800 wide; the main menu's art is 4:3.

## Modules

| Rank | Id | Module | Tier | Size | Depends on | Status |
|---|---|---|---|---|---|---|
| 0 | V0 | Output: window modes (Windowed, Windowed Borderless, Fullscreen) and Alt+Enter, size list, physical pixels, settings keys | light | 1 day | none | merged: #79, #98 |
| 1 | V1 | Sharp scaler (restore D6) + fit/integer modes + mouse mapping (restore D1's maps) | light | 1 day | V0 | merged: #82 |
| 2 | V2 | VSync On/Off and Frame Limit (restore S2/S3, add the toggle) | light | 1 day | V0 | merged: #85 |
| 3 | V3 | 16:9 frame 1068x600: camera, culling, draw grid, compositor | light | 2-3 days | none | merged: #89 |
| 4 | V4 | World live to the wider edges (rooms, unit visibility) | light | 1-2 days | V3 | merged: #94 |
| 5 | V5 | Real motion: smooth camera between ticks (restore S5, layers through the sharp scaler) | full | 1 day | V1 | merged: #81 |
| 6 | V6 | Real motion: units and missiles between ticks | full | 2-3 days | V5 | merged: #86 |
| 7 | V7 | HUD layout at 16:9 | full | 1-2 days | V3 | merged: #95, #102 |
| 8 | V8 | Panels anchored to the edges | full | 2 days | V7 | merged: #104 |
| 9 | V9 | Automap and overlays at 16:9 | full | 1 day | V3 | merged: #111 |
| 10 | V10 | Main menu at 16:9 (4:3 art centred, edges filled) | full | 1 day | V0 | merged: #88, #100 |
| 11 | V11 | UI and text at the screen's resolution, bitmap HD | full | 3-4 days | V1, V7 | merged: #115, #137, #152, #227, #256 |
| 12 | V12 | HD lighting and effects | later | to size | V11 | dropped: #267 |
| 13 | V13 | Locally generated HD sprites | later | to size | V11 | dropped: #267 |

Releases with these modules are listed in [`docs/RELEASES.md`](../../RELEASES.md).

### V0. Output

- **Window modes** (owner, 2026-10-16), the Video page's Window row:
  - **Windowed** (the default for a new settings file): a window with its border and title bar. Window Size is
    **Maximized** (the default: it fills the monitor's work area at the native resolution, since a bordered window
    the monitor's full size can't fit with the title bar and taskbar) or a fixed size from a list: 1920x1080,
    2560x1440, 3840x2160 and the 4:3 sizes, filtered to what fits the monitor;
  - **Windowed Borderless:** a borderless window covering the whole display at its native resolution (restore D3,
    no display-mode switch);
  - **Fullscreen:** exclusive fullscreen at the monitor's native resolution and current refresh rate (its current
    video mode, so the display mode doesn't change).
  - The Window Size row applies to Windowed only (greyed in the other modes).
  - Existing settings files keep their saved mode; `settings.toml` version 3 renames `borderless` to
    `windowed_borderless` and `native` to `maximized`, and older files load with the new names.
- **Alt+Enter** (in game and on the main menu) toggles the mode in use with Windowed Borderless: Windowed ↔
  Windowed Borderless, Fullscreen ↔ Windowed Borderless. The mode to return to is remembered
  (`[video] alt_enter_return`) and the toggled mode is saved as the setting.
- **Resolution row** (the play frame, applied at the next start): 16:9 (1068x600, the default), 4:3 (800x600),
  4:3 (640x480). `play --res` overrides it. Settings files older than version 3 move to 16:9 (owner: 16:9 is the
  default view everywhere).
- **Pixels:** all maths in physical pixels (Windows display scaling must not change the result).
- **Settings:** versioned `settings.toml` keys; Video page rows in game and in the main-menu options.
- **Done when:** every listed resolution presents correctly at 100, 125 and 150% Windows scaling, and the 4:3 frame
  hashes are unchanged.

### V1. Sharp scaler

- **What:**
  - restore D6's `SharpMaterial` and `sharp.wgsl`;
  - restore D1's presentation maps: fit, integer, and `to_frame`/`from_frame` for the mouse;
  - default mode Fit (sharp), with Integer as an option;
  - MSAA off on every camera (already on main).
- **Done when:**
  - capture tests show even pixel widths at 1.8x, 2.4x and 3.6x;
  - output is exactly nearest at 2x and 3x;
  - the mouse round-trips at every scale.

### V2. VSync and Frame Limit

- **VSync:**
  - On = `Fifo` (supported everywhere, the default);
  - Off = `Immediate`, falling back to `Mailbox`, then `Fifo`, from the surface's supported modes;
  - `desired_maximum_frame_latency` 1-2.
- **Frame Limit:** Off, 30, 60, 120, 144, 165, 240, 360 (360 max).
  - Restore S3's limiter, run before input, with a high-resolution timer on Windows.
  - With VSync on, limits above the refresh rate are greyed out.
  - Defaults: VSync On, Frame Limit Off (real motion uses the refresh rate).
- **Done when:** caps hold within ±1 fps (a fake-clock test plus a timed run), and VSync toggles without a restart.

### V3. 16:9 frame

- **What:** `FrameSize` gains 1068x600. The camera, world-to-screen maths, culling, the draw-order grid, the tile and
  unit passes and the compositor buffers take the frame's width.
- **Done when:**
  - 4:3 frames are byte-identical (frame hashes);
  - the world fills 1068x600;
  - an autoplay route runs in both.

### V4. World live to the wider edges

- **What:** rooms and units load and stay visible to the new edges, with no pop-in. In multiplayer the server sets
  one view size per game.
- **Owner rule:** every monster on screen is live (simulated), not only drawn. Built as a separate view ring: the
  game's `view_gap` (7 rooms at 1068 wide) sets what is live and visible, while `NEAR_GAP` (6) stays unchanged.
- **Done when:** walking each act's maps shows no side pop-in, and recorded fights play the same apart from what's
  visible.

### V5. Real motion: smooth camera

- **What:** restore S5. The compositor writes a world layer and a screen layer of each tick frame; between ticks the
  world layer slides by the fraction of the last camera step, from client-side timing over the measured tick span.
  The layers are drawn through the sharp scaler, so sub-pixel slides stay crisp at 1.8x, 2.4x and 3.6x.
- **Done when:**
  - walking at 144 Hz scrolls with no stepping;
  - tick frames are unchanged (hash tests);
  - large jumps snap.

### V6. Real motion: units and missiles

- **What:** players, monsters and missiles glide between their tick positions, from the simulation's real fixed-point
  positions (previous and current). They're drawn on a non-committing compose of the frame at display rate, at the
  frame size, so the cost doesn't grow with the screen resolution. Teleports and spawns snap.
- **Done when:**
  - movement at 144 Hz or more is smooth;
  - tick frames are unchanged;
  - the GPU cost at 2160p and 360 Hz is measured on an Intel iGPU and a discrete GPU.

### V7-V10. Layout at 16:9

- **V7, HUD:** control panel, belts and orbs centred with side fill (decide with screenshots). Owner choices for
  the side bars: plain stone pieces with every other one mirrored and a few columns blended across each join (no
  seams), no scrollwork end pieces (the stone runs to the frame edge and fades into the globe statues), gold
  border lines unbroken.
- **V8, panels:** inventory, character, skills, stash and NPC panels open at the left and right edges, with the world
  visible between them. Owner rule: at 16:9 the world always shows behind and beside open panels, never black.
- **V9, automap and overlays:** cover the full width. Built: the automap shows in every open mode that still draws
  the world (at 16:9 with both panels open it draws between them; 4:3 still hides it there), the mini map centres in
  the gap, the automap header clips to the whole frame, and the message log spans the full frame. The Esc menu, NPC
  dialogs and the hireling list stay centred boxes.
- **V10, main menu:** 4:3 art centred, edges filled. Built: every front-end screen and the loading art are placed
  and scaled like the game frame; `[video] menu_sides` (Video page row "Menu Sides", after Contrast) picks the fill:
  black (default) or a dim blur of the picture (a 40x30 CPU grid drawn as a linear-filtered sprite, no shader).
  Clicks on the fill do nothing; 4:3 windows never show it.

### V11. UI and text at the screen's resolution

- **What:** the HUD, panels and text are drawn in a second pass at the output resolution (crisp at 2160p), while the
  world stays the scaled frame. Frame hashes and hit-testing don't change; at 1x the pass is off.
- **Phase 1, the native UI pass** (`gpu_compositor::ui_pass`, `world_view::native_ui`):
  - The game frame is composed as before, UI included, so frame hashes and the frame chain don't change. The
    presented picture is the scene (the frame without its UI items), and a compute pass draws the UI items at the
    output resolution.
  - Each item covers exactly the output pixels the mouse maps into it (the inverse of `to_frame`), so hit-testing is
    unchanged and pieces side by side meet without seams. Inside an item the sharp rule applies.
  - The UI is applied as `picture · (1 − cover) + add` in palette units. Opaque, alpha (measured from the act's
    tables), additive and multiplicative draws are kept; max-component is shown as additive.
  - Used when the output is larger than the frame and `[video] native_ui` is on (the default). At 1x the frame is
    presented as it is.
  - HD seam: `world_view::ui_art::UiArt` is the one lookup point for "which image draws this UI cel at this output
    scale". An HD asset pack, loaded at run time from a local folder, answers there.
  - Captures: `play --dump-draws DIR --dump-native 3840x2160` writes the frame without and with the pass.
- **Text** is not a setting (owner and tester, 2026-10-19: the display upscaler already does it; it is not two
  separate things). Wherever the pass draws, text is drawn by it at the output resolution: each loaded glyph bitmap
  is enhanced with repeated edge-aware Scale2x passes, then its edges are smoothed at the output scale. The world
  stays crisp (the rule above is for the picture; this is the glyphs only). At 1x there is nothing to upscale
  and the frame is presented as it is. No outline fonts are shipped or searched for.
  - The front end and in-game native UI pass share the same glyph enhancer: menus, HUD, item names
    and descriptions, labels and chat use their loaded font sheets, including sheets without glyph tables.
  - Palette indices survive enhancement and pass through each draw item's existing colour maps before
    edge filtering. Layout, advances, baselines and clipping remain unchanged.
  - Enhanced glyph images are cached in memory per sheet/cel and output scale, within an eight-page
    atlas limit. Atlas exhaustion falls back to sharp bitmap glyphs. No disk cache is written.
  - The Video page has no Text row. Settings files of versions 5 to 7 may still hold `hd_text` or `text_mode`:
    they load, are ignored, and are dropped on the next write.
- **Resolution info** (owner, 2026-10-16): the Video page shows clearly which picture size, screen resolution and
  scale are in use, detected from the monitor.

  The unused Perspective row (always Off) is removed from the Video page to make room (owner, 2026-10-16).

## Testing

The display update is tested in full by the owner and testers on real hardware (monitors, GPUs and iGPUs, every
window mode and resolution) rather than through a single screenshot check (owner, 2026-10-16).

## Not planned

- **FSR and whole-picture edge-aware upscaling filters** (owner, 2026-10-16).
- **Frame prediction from rendered output:** motion comes from the simulation (V5, V6).
