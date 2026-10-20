# BUG-001: game quits at game start (GPU validation error)

- **Reported:** 2026-10-16, tester PC: Windows 10, Intel HD Graphics 630 (Vulkan), game 0.1.1.88.
- **What happens:** after creating a character and starting, the game window closes. `crash.log`: "play exited with code 1".
- **Log:** `Attachments have differing sample counts: the depth attachment's texture view has count 4 but is followed by the color attachment at index 0's texture view which has count 1` → "Quitting the application due to Validation RenderError".
- **Cause:** two cameras draw into the one window. The session camera (`app/session.rs`) and the front-end camera (`app/front_host.rs`) used Bevy's default 4x MSAA; the game's present camera has MSAA off. Strict drivers refuse the mixed sample counts.
- **Fix:** MSAA off on every camera.
- **Status:** fixed in 0.1.1.89 (#71, merged with #77).
