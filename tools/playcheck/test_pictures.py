"""Synthetic tests for the picture drivers' own arithmetic (no game, no images)."""
import unittest

import pictures


class ScreenPoint(unittest.TestCase):
    def state(self, x, y, shake=(0, 0)):
        # The camera anchor as `state` gives it: 16.16 sub-tile positions.
        return {"client": {"camera": {"x16": (x << 16) | 0x8000, "y16": (y << 16) | 0x8000,
                                      "shake": list(shake)}}}

    def test_a_tile_under_the_player_is_at_the_frame_centre(self):
        # The feet of a static unit on the player's sub-tile: the frame
        # centre, 8 px up for the unit row and 8 for the player standing in
        # the middle of its sub-tile.
        self.assertEqual(pictures.screen_point(self.state(5000, 5000), (5000, 5000)), (400, 284))

    def test_isometric_steps_and_shake(self):
        base = pictures.screen_point(self.state(5000, 5000), (5000, 5000))
        # One sub-tile along x: 16 px right, 8 px down.
        self.assertEqual(pictures.screen_point(self.state(5000, 5000), (5001, 5000)),
                         (base[0] + 16, base[1] + 8))
        # A camera shake moves the picture against it.
        self.assertEqual(pictures.screen_point(self.state(5000, 5000, (3, -2)), (5000, 5000)),
                         (base[0] - 3, base[1] + 2))


if __name__ == "__main__":
    unittest.main()
