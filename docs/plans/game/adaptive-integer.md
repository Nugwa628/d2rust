---
id: adaptive-integer
title: Adaptive Integer canvas
area: game
status: in-progress
order: 6
modules: AI1-AI7
light: AI1-AI3
depends_on: [display-v2]
---

# Adaptive Integer canvas

In progress, 2026-10-18. Queue: game, order 6 in [the plan index](../README.md).

## Goal and owner rules

Integer fills the physical window with equal, nearest-neighbour pixel blocks. Choose the
largest whole number **dividing both physical dimensions** for which screen / n is at least
800 wide and 540 tall. There is no width or height cap. Odd sizes can use n = 1.
Below either minimum, use Fit with the configured normal frame. Fit keeps its fixed frames,
scaling and frame hashes. Window resizing and live Fit ↔ Integer switches update every
consumer together. No game files or additional assets ship with this feature.

Keep adaptive selection **disabled** until AI7 integrates every module. Earlier modules provide
and test the seams but preserve the shipping Integer behavior. One independently gated PR per
module; dependent PRs can stack. Owner play checks follow headless verification.

## Current behavior

- `rules/camera.rs::FrameSize` admits three configured frames and publishes one through a
  process-wide `OnceLock`. `ui/frame.rs::Presentation` enlarges that fixed frame in Integer.
- `world_view/present.rs::WorldViewState` allocates its `FrameCycle` once. Its target image,
  `smooth.rs::SmoothLayers`, and their meshes retain the initial size. Native UI targets already
  follow the output size through `native_ui.rs::ensure_targets`.
- `ui/layout.rs::Screen::play` retains 600 UI rows for every non-low frame. HUD rows usually
  read H; side-panel art mostly reads H + sy, while inventory table rectangles retain fixed y.
- `ui/help_screen.rs` has fixed 800x600 clips and bottom callouts down to y=585.
  `OriginalUi::install` stores bounds in NPC and hire UIs.
- `app/front_host.rs` draws and maps pointers through a fixed 800x600 canvas. Screen controls
  include a main-menu Exit near the bottom and character-creation controls below row 540.
- World input derives its presentation from the current frame; OriginalUi and cached automap
  geometry also need refreshing when that frame changes.
- `d2-sim::drlg::view_gap_for_width` sees width only. `app/single_player/mod.rs` sends the
  initial gap in CreateGame and `bridge/drlg.rs` initializes the client copy once. Live changes
  must update both room sets before a wider/taller frame becomes visible.

## Ranked modules

| Rank | Id | Module | Tier | Estimate | Depends on | Status |
|---|---|---|---|---|---|---|
| 0 | AI1 | Exact frame selection and renderer resize seam | light | 30 min | — | in review |
| 1 | AI2 | Bottom-anchored HUD and full frame geometry | light | 30 min | AI1 | in review |
| 2 | AI3 | Side-panel art and hit rectangles | light | 30 min | AI2 | in review |
| 3 | AI4 | Help and overlay reflow | full | 1 day | AI2, AI3 | in review |
| 4 | AI5 | Adaptive front-end canvas | full | 1 day | AI1, AI4 | in review |
| 5 | AI6 | Pointer and automap refresh | full | 1 day | AI2, AI3, AI4, AI5 | in review |
| 6 | AI7 | Two-dimensional live room visibility and activation | full | 1 day | AI1-AI6 | open |

All prerequisites rank immediately before their first consumer. Streaming and protocol changes
rank last because they cross the client/server boundary. The light tier establishes tested,
ship-safe layout foundations; the full tier delivers the complete enabled feature.

## AI1 — Exact frame selection and renderer resize seam

Status: in review, 2026-10-18.

- **What:** select the exact frame and effective mode, and prepare index, GPU and native UI
  resources to change sizes without carrying old pixels into a new canvas. No automatic
  adaptive selection is enabled in this module.
- **Files:** `rules/adaptive_integer.rs`, `world_view/frame_resize.rs`, this plan and its index.
- **Seam:** `FrameSelection` separates planning from committing a canvas; one disabled shipping
  gate remains closed through AI6. The renderer resize API updates the frame resources.
- **Done when:** matrix selection, maximal exact divisor, undersized fallback, Fit preservation,
  Fit ↔ Integer selections and resource sizes pass headlessly. Rejected resize leaves resources
  unchanged; same-size resize preserves the persistent frame. Full d2-client gates pass.
- **Risks:** stale GPU readbacks and old layer meshes. Integration waits for the prior GPU job,
  invalidates old presentation jobs and publishes geometry only after resource replacement.

## AI2 — Bottom-anchored HUD and full frame geometry

Status: in review, 2026-10-18.

- **What:** use the actual height for the control panel, overlays, clips and control hit regions;
  represent every column, including asymmetric centering for odd widths.
- **Files:** `ui/layout.rs`, `ui/hud.rs`, `ui/hud_belt.rs`, `ui/panels/border.rs`, their tests.
- **Seam:** `Screen` made from a `FrameSize`; HUD positions derive from H.
- **Done when:** all matrix sizes retain the panel's bottom row, controls hit their drawn boxes,
  and all existing fixed-frame layout/hash tests remain unchanged.
- **Risks:** layout-box coordinates differ from frame coordinates on wide frames.

## AI3 — Side-panel art and hit rectangles

Status: in review, 2026-10-18.

- **What:** one vertical offset moves panel art, inventory grids, equipment boxes, table areas
  and hit tests together for inventory, character, skills, stash, cube, shop, quest and waypoint.
- **Files:** `ui/original.rs`, `ui/layout.rs`, `ui/panels/`, panel adapters and tests.
- **Seam:** `Screen` exposes the panel translation; table rectangles are transformed once.
- **Done when:** headless matrix tests hit the art's cells and controls for every side panel,
  including 540-row frames and live geometry replacement. Fixed layouts stay identical.
- **Risks:** applying the offset twice to already H-relative art or retaining stale table copies.

## AI4 — Help and overlay reflow

Status: in review, 2026-10-18.

- **What:** keep Help's lower callouts and close control visible; refresh all modal/fullscreen
  overlays, menus, messages, chat, tooltips and NPC/hire retained bounds.
- **Files:** `ui/help_screen.rs`, `ui/original.rs`, `ui/hire_list.rs`, `ui/npc_box.rs`, overlay tests.
- **Seam:** reflow lower Help content independently of its fixed title/close area; refresh shared
  overlay geometry with `OriginalUi` without reinstalling or losing panel state.
- **Done when:** matrix clips and close hit boxes agree with visible art; 540-row Help remains
  usable; unchanged geometry produces unchanged draws.
- **Risks:** blanket translation clips titles or makes callout text overlap.

## AI5 — Adaptive front-end canvas

Status: in review, 2026-10-18.

- **What:** main menu, character selection/creation, options and popups use the selected canvas.
  Center native-size art horizontally, re-anchor lower controls/footer and keep clicks aligned.
- **Files:** `app/front_host.rs`, `app/front_ui.rs`, `ui/front_end/`, headless front-end tests.
- **Seam:** explicit canvas geometry shared by drawing, clips, presentation and input.
- **Done when:** every screen has reachable controls at all matrix sizes, especially Exit and
  creation controls at 540 high; Fit keeps fixed 800x600 front-end hashes.
- **Risks:** old art cannot be cropped as a substitute for laying out controls.

## AI6 — Pointer and automap refresh

Status: in review, 2026-10-18.

- **What:** both pointer mappings, cursor bounds, panel jumps, and full/mini automap geometry
  follow the committed canvas, including open panels and live mode/window changes.
- **Files:** `ui/edge.rs`, `ui/original.rs`, `ui/automap/`, `world_view/automap_view.rs`, input tests.
- **Seam:** committed frame geometry, used by input and rendering in the same pass.
- **Done when:** matrix round trips target the same pixels; automap recentering, panels and odd
  edges agree after Fit ↔ Integer switches without moving the real mouse.
- **Risks:** stale camera/pointer facts on the first pass after a resize.

## AI7 — Live room visibility and activation

Status: in review, 2026-10-19 (AI7a #405 merged; AI7b activation).

- **What:** derive a conservative room range from both dimensions, synchronize client/server
  room membership on changes and then enable adaptive selection in the shipping path.
- **Files:** `d2-sim/src/drlg/`, `d2-proto` source schema, `d2-server`, `bridge/drlg.rs`,
  `app/single_player/`, `world_view/present.rs`, `rules/adaptive_integer.rs`.
- **Seam:** a live view update carried to the authority and acknowledged before publishing a
  larger canvas. Generated protocol code is rebuilt from its source of truth.
- **Done when:** widest and tallest matrix frames are covered at every edge by room visibility;
  client/server room sets agree after live resizing and Fit ↔ Integer; headless integration and
  touched-crate gates pass; regenerated output is clean. Only then open the shipping gate.
- **Risks:** missing rooms or divergent memberships if startup-only gaps are reused; stale GPU
  jobs may not commit into a different-size cycle. Preserve determinism and Fit baselines.

## Owner play checks

After activation, inspect HUD, every panel, Help, menus, cursor, full/mini automap and room edges
at 1080p and 4K first, then 1440p, ultrawide, tall and odd windows. Resize with panels open and
switch Fit ↔ Integer while playing. Check low-spec memory/performance. No lane play tests or
network tests are authorized; all automated acceptance is headless.

## Not planned

- Filtering or fractional Integer scaling: exact nearest-neighbour multiples are the contract.
- A width cap or restricted field of view: the owner allows wider and taller views.
- A separate adaptive setting: Integer selects this behavior after the full path is complete.
