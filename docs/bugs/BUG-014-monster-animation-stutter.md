# BUG-014: monster animations stutter too (fixed, stepped frames)

- **Reported:** 2026-10-16, test build 0.1.1.91 (owner). Setup: 16:9, Fullscreen, 1920x1080.
- **What happens:** monsters move and animate in fixed steps, not smoothly, the same as the player character.
- **Expected:** units glide between ticks (V6) and animate at their normal rate.
- **Notes:** this probably shares a cause with BUG-010 (only about 5 ticks a second, and between-tick drawing not running). Check it again once BUG-010 is fixed.
- **Status:** open.
