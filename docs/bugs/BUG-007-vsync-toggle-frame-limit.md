# BUG-007: VSync toggle, "Frame Limit" naming

- **Reported:** 2026-10-16 (owner request).
- **Change:**
  - VSync becomes a setting the player can turn on and off (it was always on).
  - The "Frame Cap" row is renamed "Frame Limit".
  - Its "Monitor" value is renamed "VSync" (limit to the monitor's refresh rate).
- **Owner rule update:** this replaces the earlier rule "vsync always on, can't be disabled" (`docs/plans/game/smoothness-update.md`, S2). 360 fps stays the maximum.
- **Status:** fixed in 0.1.1.90: VSync On/Off and the Frame Limit row (V2, #85).
