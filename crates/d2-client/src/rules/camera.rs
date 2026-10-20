//! From a world position to a screen draw position.
//!
//! The camera works in *client pixels*: the isometric plane every unit and tile is placed
//! on ([`UnitPosition::client`], [`cell_origin`]). Each drawn frame has one [`Camera`],
//! computed once from the local player's position, the open side panels and the screen
//! shake; it gives the draw positions of units and tiles and decides which floor, roof and
//! wall pieces are inside the view. This module also holds the frame sizes the game can
//! run at, the play area above the control panel, and the screen-shake envelope.
//!
//! Integer math only. Divisions are on positive sizes and the shifts on signed pixel sums
//! are arithmetic (floor), so every position is exact and the same on every machine.

use d2_sim::rng::Seed;

use crate::scene::Rect;

/// Rows of the control panel below the play area: the play area is `H − 40` rows high.
pub const PANEL_HEIGHT: i32 = 40;
/// Client tick in milliseconds.
pub const TICK_MS: u32 = 40;
/// The client tick in milliseconds, the one constant every client clock
/// reads (D2RUST's own clock, unverified; `q-fix-prov-frame-clock`).
pub const CLIENT_TICK_MS: u32 = TICK_MS;
/// Half the width of a tile cell in client pixels: `sx = (tx − ty) × 80`.
pub const CELL_HALF_WIDTH: i32 = 80;
/// Half the height of a tile cell in client pixels: `sy = (tx + ty) × 40`.
pub const CELL_HALF_HEIGHT: i32 = 40;

/// The frame W × H the world is drawn into. Three frames exist: 800 × 600
/// (the default), 640 × 480 (`play --res 640x480`) and the 16:9 frame
/// 1068 × 600 (`play --res 1068x600`). Every rule here reads the size, so
/// the camera, culling and draw grid follow the frame's width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameSize {
    pub width: i32,
    pub height: i32,
}

/// Width of the box the HUD and panels are laid out in (the 800 × 600
/// layout). A wider frame centres that box ([`FrameSize::ui_left`]).
pub const UI_BOX_WIDTH: i32 = 800;

/// Width of one side panel (character, inventory, stash, …).
pub const SIDE_PANEL_WIDTH: i32 = 320;

/// [`FrameSize::side_panel_span`] of a frame `width` columns wide: half of
/// a 4:3 frame, the panel's own 320 columns in a wider one.
pub fn side_panel_span(width: i32) -> i32 {
    if width > UI_BOX_WIDTH {
        SIDE_PANEL_WIDTH
    } else {
        width / 2
    }
}

impl FrameSize {
    /// The 800 × 600 frame (resolution mode 2), the default.
    pub const D2RUST: FrameSize = FrameSize {
        width: 800,
        height: 600,
    };
    /// The 640 × 480 frame (resolution mode 0, `play --res 640x480`).
    pub const LOW: FrameSize = FrameSize {
        width: 640,
        height: 480,
    };
    /// The 16:9 frame (`play --res 1068x600`): the 800 × 600 frame's 600
    /// lines with 268 more columns of world, 134 on each side. It is
    /// within 0.2 % of 16:9 (an even width, 4-byte rows) and scales by
    /// 1.8, 2.4 and 3.6 to 1080p, 1440p and 2160p.
    pub const WIDE: FrameSize = FrameSize {
        width: 1068,
        height: 600,
    };

    /// The frames `play` can run.
    pub const ALL: [FrameSize; 3] = [FrameSize::D2RUST, FrameSize::LOW, FrameSize::WIDE];

    /// The frame named `WxH` on the command line (`800x600`, `640x480`,
    /// `1068x600`); `None` for any other text.
    pub fn parse(s: &str) -> Option<FrameSize> {
        Self::ALL
            .into_iter()
            .find(|f| s == format!("{}x{}", f.width, f.height))
    }

    /// The resolution mode the layout rules read: 0 for 640 × 480, 2 for
    /// the 600-line frames (800 × 600, and the 16:9 frame, which shares
    /// its height and its UI layout).
    pub fn resolution_mode(&self) -> u32 {
        if *self == Self::LOW {
            0
        } else {
            2
        }
    }

    /// Left edge of the HUD and panel box in the frame: 0 at 4:3, and
    /// `(W − 800) / 2` (134) at 16:9, so the 800-wide UI layout stays
    /// centred while the world fills the whole width.
    pub fn ui_left(&self) -> i32 {
        (self.width - UI_BOX_WIDTH).max(0) / 2
    }

    /// The columns one open side panel takes from the frame, counted from
    /// its outer edge. At 4:3 that is half the frame (the 320-wide panel
    /// and, at 800 × 600, its 80-wide stone border). A wider frame opens
    /// its panels at its own edges ([`crate::ui::layout::Screen::sx`]), so
    /// a panel takes only its own 320 columns and the world shows between
    /// the two sides.
    pub fn side_panel_span(&self) -> i32 {
        side_panel_span(self.width)
    }

    /// Whether a frame in open mode `mode` draws the world. With both side
    /// panels open (mode 3) a 4:3 frame is covered and draws none; a wider
    /// frame still shows the world between the two panels.
    pub fn draws_world(&self, mode: OpenMode) -> bool {
        mode.get() != 3 || 2 * self.side_panel_span() < self.width
    }

    /// Play-area height: the frame minus the control panel, `H − 40`.
    pub fn play_height(&self) -> i32 {
        self.height - PANEL_HEIGHT
    }

    /// `H − 49`: a press at this mouse y or below no longer makes a skill code (the
    /// bottom edge of the area where clicks reach the world; hovering uses the same
    /// bound).
    pub fn skill_y_limit(&self) -> i32 {
        self.height - 0x31
    }

    /// The frame `[0, W) × [0, H)`: the clip of every world draw.
    pub fn rect(&self) -> Rect {
        Rect::new(0, 0, self.width as u32, self.height as u32)
    }

    /// The frame the play path draws. [`FrameSize::set_play`] chooses one of
    /// the configured frames before the app starts; adaptive selection can
    /// then commit another size with [`FrameSize::set_play_live`]. Defaults
    /// to [`FrameSize::D2RUST`].
    pub fn play() -> FrameSize {
        PLAY_FRAME
            .get()
            .map(|f| *f.read().unwrap_or_else(|e| e.into_inner()))
            .unwrap_or(FrameSize::D2RUST)
    }

    /// The play frame [`FrameSize::set_play`] or [`FrameSize::set_play_live`]
    /// chose, if it was called.
    pub fn play_chosen() -> Option<FrameSize> {
        PLAY_FRAME
            .get()
            .map(|f| *f.read().unwrap_or_else(|e| e.into_inner()))
    }

    /// The configured normal frame adaptive selection falls back to. Defaults
    /// to the last [`FrameSize::set_play`] frame, or [`FrameSize::D2RUST`].
    pub fn normal_play() -> FrameSize {
        PLAY_NORMAL.get().copied().unwrap_or_else(FrameSize::play)
    }

    /// The normal frame a caller configured, if any.
    pub fn normal_chosen() -> Option<FrameSize> {
        PLAY_NORMAL.get().copied()
    }

    /// Records the configured normal frame without changing a live play
    /// frame.
    pub fn set_normal_play(size: FrameSize) -> Result<(), PlayFrameError> {
        if !Self::ALL.contains(&size) {
            return Err(PlayFrameError::Unsupported(size));
        }
        let _ = PLAY_NORMAL.set(size);
        Ok(())
    }

    /// Chooses the play frame (one of [`FrameSize::ALL`]) once per process;
    /// a second call with another size is refused. This also records the
    /// configured normal frame.
    pub fn set_play(size: FrameSize) -> Result<(), PlayFrameError> {
        if !Self::ALL.contains(&size) {
            return Err(PlayFrameError::Unsupported(size));
        }
        let _ = PLAY_NORMAL.set(size);
        let frame = PLAY_FRAME.get_or_init(|| std::sync::RwLock::new(size));
        let now = *frame.read().unwrap_or_else(|e| e.into_inner());
        if now != size {
            return Err(PlayFrameError::AlreadySet { now, asked: size });
        }
        Ok(())
    }

    /// Publishes a live adaptive frame. The caller has already committed the
    /// renderer, UI and room streaming to this size.
    pub fn set_play_live(size: FrameSize) -> Result<(), PlayFrameError> {
        if size.width <= 0 || size.height <= crate::scene::UNCLEARED_ROWS as i32 {
            return Err(PlayFrameError::Invalid(size));
        }
        let frame = PLAY_FRAME.get_or_init(|| std::sync::RwLock::new(size));
        *frame.write().unwrap_or_else(|e| e.into_inner()) = size;
        Ok(())
    }
}

/// The play frame of [`FrameSize::play`].
static PLAY_FRAME: std::sync::OnceLock<std::sync::RwLock<FrameSize>> = std::sync::OnceLock::new();

/// The configured normal frame adaptive selection returns to.
static PLAY_NORMAL: std::sync::OnceLock<FrameSize> = std::sync::OnceLock::new();

/// Errors of [`FrameSize::set_play`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlayFrameError {
    #[error("frame {0:?} is not one of 800 × 600, 640 × 480 or 1068 × 600")]
    Unsupported(FrameSize),
    #[error("frame {0:?} is not a valid live canvas")]
    Invalid(FrameSize),
    #[error("the play frame is already {now:?}; {asked:?} refused")]
    AlreadySet { now: FrameSize, asked: FrameSize },
}

/// Which side panels are open: 0 none, 1 a right panel, 2 a left panel, 3 both. The UI
/// sets it as panels open and close; the camera shifts the view for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct OpenMode(u8);

impl OpenMode {
    /// No side panel open.
    pub const NONE: OpenMode = OpenMode(0);

    /// `None` outside 0–3.
    pub fn new(mode: u8) -> Option<Self> {
        (mode <= 3).then_some(OpenMode(mode))
    }

    pub fn get(&self) -> u8 {
        self.0
    }
}

/// The view rectangle and the horizontal shift an open side panel gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    /// How far the view moves sideways for an open side panel (negative: left).
    pub shift_x: i32,
}

impl ViewRect {
    /// The view for frame `size` and open mode `mode`. With one side
    /// panel open (mode 1: right, mode 2: left) the view moves by half the
    /// columns the panel takes ([`FrameSize::side_panel_span`]), so the
    /// player stays centred in the world left visible: `W / 4` at 4:3, 160
    /// in the 16:9 frame. Both sides open (mode 3) or none: no move.
    pub fn new(size: FrameSize, mode: OpenMode) -> Self {
        let half = size.side_panel_span() / 2;
        let shift_x = match mode.0 {
            1 => -half,
            2 => half,
            _ => 0,
        };
        ViewRect {
            left: shift_x,
            top: 0,
            right: size.width + shift_x,
            bottom: size.play_height(),
            shift_x,
        }
    }
}

/// A position in client pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ClientPos {
    pub x: i32,
    pub y: i32,
}

/// A unit position as the client keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnitPosition {
    /// A moving unit (players, monsters, missiles): 16.16 fixed subtiles,
    /// unsigned.
    Moving { x16: u32, y16: u32 },
    /// A static unit (objects, items, tiles; unit types 2, 4, 5): integer
    /// subtiles.
    Static { sx: i32, sy: i32 },
}

impl UnitPosition {
    /// The unit's client pixel position.
    pub fn client(&self) -> ClientPos {
        match *self {
            UnitPosition::Moving { x16, y16 } => moving_to_client(x16, y16),
            UnitPosition::Static { sx, sy } => static_to_client(sx, sy),
        }
    }
}

/// Client pixels of a moving unit: `a = x16 >> 11`, `b = y16 >> 11` (logical), then
/// `px = (a − b) >> 1`, `py = (a + b) >> 2`. The last shifts are arithmetic, so negative
/// sums round down (floor), never toward zero.
pub fn moving_to_client(x16: u32, y16: u32) -> ClientPos {
    // < 2^21 each, so the sums fit an i32.
    let a = (x16 >> 11) as i32;
    let b = (y16 >> 11) as i32;
    ClientPos {
        x: (a - b) >> 1,
        y: (a + b) >> 2,
    }
}

/// Client pixels of a static unit: `px = (sx − sy) × 16`, `py = (sx + sy) × 8`.
pub fn static_to_client(sx: i32, sy: i32) -> ClientPos {
    ClientPos {
        x: (sx - sy) * 16,
        y: (sx + sy) * 8,
    }
}

/// The client origin `(sx, sy)` of tile cell `(tx, ty)`.
pub fn cell_origin(tx: i32, ty: i32) -> ClientPos {
    ClientPos {
        x: (tx - ty) * CELL_HALF_WIDTH,
        y: (tx + ty) * CELL_HALF_HEIGHT,
    }
}

/// The position a tile record stores for cell `(tx, ty)`: `(sx − 80, sy + 80)`.
pub fn tile_entry(tx: i32, ty: i32) -> (i32, i32) {
    let c = cell_origin(tx, ty);
    (c.x - CELL_HALF_WIDTH, c.y + 2 * CELL_HALF_HEIGHT)
}

/// Which tile list a tile is drawn from. Which orientations go in which list is decided
/// by [`super::draw_order`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TileList {
    Floor,
    Wall,
    /// A roof, raised by its DT1 tile's roof height.
    Roof {
        roof_height: i32,
    },
}

/// The inputs of the frame's one camera besides the open mode: the local player's
/// position and the shake offsets, decided once per frame. Every reader that builds a
/// camera or places the local player takes this one, so all of them agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameAnchor {
    /// The local player's position (the walk prediction in `play`).
    pub player: UnitPosition,
    /// The screen-shake offsets ([`shake_offsets`]), `(0, 0)` when no shake runs.
    pub shake: (i32, i32),
}

impl FrameAnchor {
    /// The frame's camera in open mode `mode`.
    pub fn camera(&self, size: FrameSize, mode: OpenMode) -> Camera {
        Camera::new(size, mode, self.player.client(), self.shake)
    }
}

/// The camera of one drawn frame: computed once, from the local player's client
/// position, the open mode and the frame's shake offsets.
///
/// It has two origins. Tiles are placed relative to the centre of the view rectangle,
/// units relative to the centre of the whole frame (16 pixels lower), and units then add
/// the panel shift themselves ([`Camera::unit_draw`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Camera {
    pub size: FrameSize,
    pub view: ViewRect,
    /// Tile origin `(cx_t, cy_t)`.
    pub tile: ClientPos,
    /// Unit origin `(cx_u, cy_u)`.
    pub unit: ClientPos,
}

impl Camera {
    /// The camera centred on `player`. `shake` is the frame's shake offsets, `(0, 0)` when
    /// no shake runs.
    pub fn new(size: FrameSize, mode: OpenMode, player: ClientPos, shake: (i32, i32)) -> Self {
        let view = ViewRect::new(size, mode);
        let (dx, dy) = shake;
        Camera {
            size,
            view,
            tile: ClientPos {
                x: player.x - (view.right - view.left) / 2 + dx,
                y: player.y - (view.bottom - view.top) / 2 + dy,
            },
            unit: ClientPos {
                x: player.x - size.width / 2 + dx,
                y: player.y - size.height / 2 + 16 + dy,
            },
        }
    }

    /// The draw position (X, Y) of a unit at client `at` with the per-unit extra offsets
    /// `(ox, oy)` ([`super::unit_composite`]).
    pub fn unit_draw(&self, at: ClientPos, extra: (i32, i32)) -> (i32, i32) {
        (
            at.x + extra.0 - self.unit.x + self.view.shift_x,
            at.y + extra.1 - self.unit.y + 8,
        )
    }

    /// The (X, Y) handed to the tile drawer for cell `(tx, ty)` in `list`.
    pub fn tile_handed(&self, list: TileList, tx: i32, ty: i32) -> (i32, i32) {
        self.handed_at(list, cell_origin(tx, ty))
    }

    /// [`Camera::tile_handed`] for a cell whose client origin is `s = (sx, sy)`.
    pub fn handed_at(&self, list: TileList, s: ClientPos) -> (i32, i32) {
        let (cx, cy) = (self.tile.x, self.tile.y);
        match list {
            TileList::Floor => (s.x - cx, s.y - cy),
            TileList::Wall => (
                s.x - CELL_HALF_WIDTH - cx + self.view.left,
                s.y + 2 * CELL_HALF_HEIGHT - cy + self.view.top,
            ),
            TileList::Roof { roof_height } => (s.x - cx, s.y - roof_height - cy),
        }
    }

    /// Where block pixel `(0, 0)` of a tile handed `(x, y)` lands.
    /// The floor drawer (also used for roofs) moves X by −80 and by the
    /// panel shift `left`; the wall drawer takes (X, Y) as is.
    pub fn block_origin(&self, list: TileList, handed: (i32, i32)) -> (i32, i32) {
        match list {
            TileList::Floor | TileList::Roof { .. } => {
                (handed.0 - CELL_HALF_WIDTH + self.view.left, handed.1)
            }
            TileList::Wall => handed,
        }
    }

    /// Whether a floor or roof handed `(x, y)` is drawn: inside the view clip rectangle
    /// `[−80, W + 80) × [−80, H − 47)`.
    pub fn floor_roof_visible(&self, handed: (i32, i32)) -> bool {
        let (x, y) = handed;
        (-80..self.size.width + 80).contains(&x) && (-80..self.size.height - 47).contains(&y)
    }

    /// Whether a wall block at screen `(x, y)` (its top-left) is drawn:
    /// x in `[−32, W)` (modes 0/3); with one side panel open, walls under
    /// the panel are skipped: `[−32, W − S)` (mode 1, right panel) or
    /// `[S − 32, W)` (mode 2, left panel), S the panel's columns
    /// ([`FrameSize::side_panel_span`]: `W / 2` at 4:3, so in a wider frame
    /// the world beside the panel keeps its walls); y in `[−32, H + 32)`.
    pub fn wall_block_visible(&self, x: i32, y: i32) -> bool {
        let (w, span) = (self.size.width, self.size.side_panel_span());
        let xs = match self.view.shift_x {
            s if s < 0 => -32..w - span,
            s if s > 0 => span - 32..w,
            _ => -32..w,
        };
        xs.contains(&x) && (-32..self.size.height + 32).contains(&y)
    }
}

/// A running screen shake: peak amplitude `A`, then attack `t1`, sustain `t2` and release
/// `t3`, in milliseconds. The amplitude ramps up over the attack, holds through the
/// sustain and ramps down over the release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shake {
    pub peak: u32,
    pub attack: u32,
    pub sustain: u32,
    pub release: u32,
}

impl Shake {
    /// `None` when `t2 = 0`: a shake without sustain never starts.
    pub fn start(peak: u32, attack: u32, sustain: u32, release: u32) -> Option<Self> {
        (sustain != 0).then_some(Shake {
            peak,
            attack,
            sustain,
            release,
        })
    }

    /// The amplitude `a` at `t` ms after the start, or `None` once `t > t1 + t2 + t3`
    /// (the shake has ended; offsets 0). Every value is unsigned 32-bit: sums and the
    /// product keep their low 32 bits, comparisons and the division are unsigned. A zero divisor is never reached: the attack row needs
    /// `t < t1`, and with `t3 = 0` the release row (only `t = t1 + t2`)
    /// gives `a = 0`.
    pub fn amplitude(&self, t: u32) -> Option<u32> {
        let (a, t1, t2, t3) = (self.peak, self.attack, self.sustain, self.release);
        let end = t1.wrapping_add(t2).wrapping_add(t3);
        if t > end {
            return None;
        }
        Some(if t < t1 {
            a.wrapping_mul(t) / t1
        } else if t < t1.wrapping_add(t2) {
            a
        } else {
            // t3 = 0: only t = t1 + t2 reaches this row, a = 0.
            a.wrapping_mul(end - t).checked_div(t3).unwrap_or(0)
        })
    }

    /// The D2RUST envelope time of the frame `ticks` client ticks after
    /// the start (`t = 40 × ticks`).
    pub fn time_of(ticks: u32) -> u32 {
        Self::time_at(ticks, d2_sim::tick::TickRate::BASE)
    }

    pub fn time_at(ticks: u32, rate: d2_sim::tick::TickRate) -> u32 {
        rate.ms_for_ticks(ticks) as u32
    }
}

/// The frame's shake offsets `(dx, dy)` for amplitude `a`, drawn from the local player
/// unit's seed: `dx = roll_range(−a, 2a)`, then `dy` the same. `a = 0` draws nothing from
/// the seed and gives `(0, 0)`.
pub fn shake_offsets(a: u32, seed: &mut Seed) -> (i32, i32) {
    if a == 0 {
        return (0, 0);
    }
    let a = a as i32;
    let n = a.wrapping_mul(2);
    let dx = seed.roll_range(a.wrapping_neg(), n);
    let dy = seed.roll_range(a.wrapping_neg(), n);
    (dx, dy)
}

#[cfg(test)]
mod frame_tests {
    use super::*;
    use crate::rules::draw_order::{tile_of, DrawGrid};

    #[test]
    fn the_wide_frame_is_16_9_with_the_ui_box_centred() {
        let w = FrameSize::WIDE;
        assert_eq!((w.width, w.height), (1068, 600));
        // 16:9 is 1066.7 × 600: within 0.2 %.
        let ratio = f64::from(w.width) / f64::from(w.height);
        assert!((ratio / (16.0 / 9.0) - 1.0).abs() < 0.002);
        assert_eq!(w.ui_left(), 134);
        assert_eq!(w.ui_left() * 2 + UI_BOX_WIDTH, w.width);
        assert_eq!(FrameSize::D2RUST.ui_left(), 0);
        assert_eq!(FrameSize::LOW.ui_left(), 0);
        assert_eq!(w.resolution_mode(), 2);
        assert_eq!(w.play_height(), FrameSize::D2RUST.play_height());
        assert!(FrameSize::ALL.contains(&w));
    }

    /// The 16:9 camera is the 800 × 600 camera moved 134 px right: the
    /// player stays at the centre, and every unit and tile lands 134 px
    /// further right, so the extra width is world on both sides.
    #[test]
    fn the_wide_camera_is_the_4_3_camera_widened_on_both_sides() {
        let player = ClientPos {
            x: 12_345,
            y: 6_789,
        };
        let mode = OpenMode::NONE;
        let wide = Camera::new(FrameSize::WIDE, mode, player, (3, -2));
        let base = Camera::new(FrameSize::D2RUST, mode, player, (3, -2));
        assert_eq!(wide.view.right - wide.view.left, 1068);
        assert_eq!(wide.unit_draw(player, (0, 0)), (534 - 3, 292 + 2));
        for at in [
            ClientPos { x: 0, y: 0 },
            ClientPos {
                x: 12_000,
                y: 7_000,
            },
        ] {
            let (wx, wy) = wide.unit_draw(at, (5, 7));
            let (bx, by) = base.unit_draw(at, (5, 7));
            assert_eq!((wx - bx, wy - by), (134, 0));
        }
        for list in [
            TileList::Floor,
            TileList::Wall,
            TileList::Roof { roof_height: 80 },
        ] {
            let (wx, wy) = wide.tile_handed(list, 40, 70);
            let (bx, by) = base.tile_handed(list, 40, 70);
            assert_eq!((wx - bx, wy - by), (134, 0), "{list:?}");
        }
    }

    /// Culling keeps what lies anywhere across the 1068 columns.
    #[test]
    fn the_wide_frame_culls_at_its_own_edges() {
        let p = ClientPos { x: 0, y: 0 };
        let wide = Camera::new(FrameSize::WIDE, OpenMode::NONE, p, (0, 0));
        let base = Camera::new(FrameSize::D2RUST, OpenMode::NONE, p, (0, 0));
        assert!(wide.floor_roof_visible((1067 + 80, 0)));
        assert!(!wide.floor_roof_visible((1068 + 80, 0)));
        assert!(!base.floor_roof_visible((1067 + 80, 0)));
        assert!(wide.wall_block_visible(1067, 0));
        assert!(!wide.wall_block_visible(1068, 0));
        assert!(!base.wall_block_visible(1067, 0));
        // A right panel open: the world shows up to the panel's first
        // column, 1068 − 320.
        let right = Camera::new(FrameSize::WIDE, OpenMode::new(1).unwrap(), p, (0, 0));
        assert_eq!(right.view.shift_x, -160);
        assert!(right.wall_block_visible(747, 0));
        assert!(!right.wall_block_visible(748, 0));
    }

    /// An open side panel moves the view by half the columns it takes and
    /// skips the walls under it. At 4:3 a panel side is half the frame
    /// (W / 4 move, walls from W / 2); the 16:9 frame opens its panels at
    /// its edges, so a panel takes 320 columns (160 move), and the world
    /// beside it, the outer strips included, keeps its walls.
    #[test]
    fn a_side_panel_moves_the_view_by_half_its_columns() {
        let p = ClientPos { x: 0, y: 0 };
        let cam = |size, mode| Camera::new(size, OpenMode::new(mode).unwrap(), p, (0, 0));
        for (size, span) in [
            (FrameSize::D2RUST, 400),
            (FrameSize::LOW, 320),
            (FrameSize::WIDE, 320),
        ] {
            assert_eq!(size.side_panel_span(), span);
            assert_eq!(cam(size, 0).view.shift_x, 0);
            assert_eq!(cam(size, 1).view.shift_x, -span / 2);
            assert_eq!(cam(size, 2).view.shift_x, span / 2);
            assert_eq!(cam(size, 3).view.shift_x, 0);
            // Left panel open: walls from the panel's edge on.
            assert!(!cam(size, 2).wall_block_visible(span - 33, 0));
            assert!(cam(size, 2).wall_block_visible(span - 32, 0));
            // Right panel open: walls up to the panel's first column.
            let w = size.width;
            assert!(cam(size, 1).wall_block_visible(w - span - 1, 0));
            assert!(!cam(size, 1).wall_block_visible(w - span, 0));
        }
        // The 4:3 rule as before: W / 4 and the W / 2 wall bound.
        assert_eq!(cam(FrameSize::D2RUST, 2).view.shift_x, 800 / 4);
        assert_eq!(cam(FrameSize::LOW, 1).view.shift_x, -(640 / 4));
        // The world is drawn in every open mode of the 16:9 frame (both
        // panels leave 428 columns between them); at 4:3 mode 3 covers it.
        for m in 0..=3 {
            let mode = OpenMode::new(m).unwrap();
            assert!(FrameSize::WIDE.draws_world(mode));
            for size in [FrameSize::D2RUST, FrameSize::LOW] {
                assert_eq!(size.draws_world(mode), m != 3);
            }
        }
        // In the 16:9 frame the world between a panel and the frame's
        // middle keeps its walls (a W / 2 bound would cull them).
        assert!(cam(FrameSize::WIDE, 2).wall_block_visible(500, 0));
        assert!(cam(FrameSize::WIDE, 1).wall_block_visible(600, 0));
    }

    /// Every pixel of the play area falls in a cell of the frame's draw
    /// grid, at 4:3 and at 16:9: the grid's side follows the width.
    #[test]
    fn the_draw_grid_covers_the_whole_frame() {
        for size in FrameSize::ALL {
            let cam = Camera::new(
                size,
                OpenMode::NONE,
                ClientPos { x: 4_000, y: 9_000 },
                (0, 0),
            );
            let grid = DrawGrid::of_camera(&cam);
            assert_eq!(grid.view.right - grid.view.left, size.width);
            for y in (0..size.play_height()).step_by(7) {
                for x in (0..size.width).step_by(5).chain([size.width - 1]) {
                    let t = tile_of(cam.tile.x + x, cam.tile.y + y);
                    assert!(grid.cell(t).is_some(), "{size:?}: pixel ({x}, {y})");
                }
            }
        }
    }
}
