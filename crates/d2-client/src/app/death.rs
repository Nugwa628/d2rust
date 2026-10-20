//! The local player's death in the play preview: the death screen text while
//! the player is dying or dead, and Esc as the respawn path (the escape menu
//! does not open for a dead player; Esc sends C→S 0x41 and returns 0).
//!
//! The server owns the outcome (it starts DT when the life reaches 0,
//! makes the corpse, and answers 0x41 with the respawn in town,
//! `d2_server::adapters::handlers::player::resurrect`); this module only
//! shows the state and sends the intent.
//!
//! The screen is the game's: the
//! two banner images and up to two text rows. The banners are DC6 art in
//! the game; the preview sets their words as text.

use bevy::prelude::*;

use crate::bridge::modes::player_mode;
use crate::bridge::world::ClientWorld;
use crate::bridge::BridgeResource;

use super::hardcore::HardcoreRun;

/// The words of the two banners (`youdiedhardcore.dc6`, `youdiedinst.dc6`).
pub const BANNER_TITLE: &str = "YOU HAVE DIED";
pub const BANNER_INST: &str = "PRESS ESC TO CONTINUE.";

/// `string.tbl` 5094 `Deathmessage` (`%d` = the gold lost), 5095
/// `Deathmessnight`, 5096 `Harddeathmessage`.
const GOLD_LOST: &str = "Death takes its toll of %d Gold";
const EXP_LOST: &str = "You have lost experience";
const HARDCORE_MSG: &str = "Your deeds of valor will be remembered";

/// The stat the death fills with the gold it took (`goldlost`, 175).
const STAT_GOLD_LOST: u16 = 175;

/// A text row of the screen and its baseline's offset from the middle of
/// the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeathRow {
    pub y: i32,
    pub text: String,
}

/// The text rows under the banners: row 1 at H / 2 + 2 (the hardcore
/// message, or the gold lost when non-zero), row 2 (above Normal, not
/// hardcore) 48 lower, or at row 1's place when row 1 is absent.
pub fn death_rows(hardcore: bool, gold_lost: i32, difficulty: u8) -> Vec<DeathRow> {
    let mut rows = Vec::new();
    let mut y = 2;
    if hardcore {
        rows.push(DeathRow {
            y,
            text: HARDCORE_MSG.to_owned(),
        });
    } else if gold_lost != 0 {
        rows.push(DeathRow {
            y,
            text: GOLD_LOST.replace("%d", &gold_lost.to_string()),
        });
        y += 0x30;
    }
    if difficulty > 0 && !hardcore {
        rows.push(DeathRow {
            y,
            text: EXP_LOST.to_owned(),
        });
    }
    rows
}

/// The C→S 0x41 Resurrect (1 byte).
pub const RESURRECT: [u8; 1] = [0x41];

/// Whether the death screen is up.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeathScreen {
    pub active: bool,
}

/// One text node of the death screen: 0 the title banner, 1 the
/// instruction banner, 2 and 3 the text rows.
#[derive(Component)]
pub struct DeathText(pub usize);

/// The local player is dying (mode 0) or dead (mode 0x11).
pub fn is_down(w: &ClientWorld) -> bool {
    w.local()
        .is_some_and(|u| u.mode == player_mode::DEATH || u.mode == player_mode::DEAD)
}

/// The local player lies dead (mode 0x11): the only mode the server
/// accepts 0x41 in.
pub fn is_dead(w: &ClientWorld) -> bool {
    w.local().is_some_and(|u| u.mode == player_mode::DEAD)
}

/// Banner bottoms (H / 2 − 94 and H / 2 − 46, 36 high) as tops relative to the
/// middle of the window.
const ROW_TOPS: [i32; 4] = [-130, -82, -14, -14];

fn spawn_text(mut commands: Commands) {
    #[allow(clippy::needless_range_loop)]
    for i in 0..4usize {
        let size = if i < 2 { 28.0 } else { 16.0 };
        let top = ROW_TOPS[i];
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    top: Val::Percent(50.0),
                    margin: UiRect::top(Val::Px(top as f32)),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                Visibility::Hidden,
                DeathText(i),
                crate::app::session::GameEntity,
            ))
            .with_children(|p| {
                p.spawn((
                    Text::new(""),
                    TextFont {
                        font_size: bevy::text::FontSize::Px(size),
                        ..default()
                    },
                    TextColor(if i < 2 {
                        Color::srgb(0.75, 0.65, 0.3)
                    } else {
                        Color::WHITE
                    }),
                ));
            });
    }
}

/// Shows the screen while the player is down; Esc while dead sends 0x41.
pub(super) fn death_screen(
    mut bridge: ResMut<BridgeResource>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    hardcore: Option<Res<HardcoreRun>>,
    mut screen: ResMut<DeathScreen>,
    mut nodes: Query<(&DeathText, &mut Visibility, &mut Node, &Children)>,
    mut texts: Query<&mut Text>,
) {
    let w = bridge.0.world();
    let down = is_down(w);
    let respawn = is_dead(w) && keys.is_some_and(|k| k.just_pressed(KeyCode::Escape));
    screen.active = down;
    let rows = match w.local_player {
        Some(key) if down => death_rows(
            hardcore.is_some_and(|h| h.0),
            w.total(key, STAT_GOLD_LOST, 0),
            w.difficulty,
        ),
        _ => Vec::new(),
    };
    for (n, mut vis, mut node, kids) in &mut nodes {
        let text = match n.0 {
            0 => Some(BANNER_TITLE.to_owned()),
            1 => Some(BANNER_INST.to_owned()),
            i => rows.get(i - 2).map(|r| {
                node.margin = UiRect::top(Val::Px((r.y - 16) as f32));
                r.text.clone()
            }),
        };
        *vis = if down && text.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if let (Some(t), Some(kid)) = (text, kids.first()) {
            if let Ok(mut tx) = texts.get_mut(*kid) {
                if tx.0 != t {
                    tx.0 = t;
                }
            }
        }
    }
    if respawn {
        if let Err(e) = bridge.0.send_bytes(&RESURRECT) {
            warn!("respawn request refused: {e}");
        }
    }
}

/// The systems of the wiring below, once per app (`app::session`).
pub fn register_death(app: &mut App) {
    app.add_systems(
        Update,
        (
            spawn_text.run_if(resource_added::<DeathScreen>),
            death_screen
                .run_if(resource_exists::<BridgeResource>)
                .run_if(resource_exists::<DeathScreen>),
        )
            .chain(),
    );
}

/// Installs the death screen and the respawn key.
pub fn add_death(app: &mut impl super::session::Setup) {
    app.register("death", register_death);
    app.world_mut().insert_resource(DeathScreen::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_follow_the_original_rules() {
        let row = |y, t: &str| DeathRow {
            y,
            text: t.to_owned(),
        };
        // Normal, gold lost: one row.
        assert_eq!(
            death_rows(false, 120, 0),
            vec![row(2, "Death takes its toll of 120 Gold")]
        );
        // Nightmare, gold lost: the experience row 48 lower.
        assert_eq!(
            death_rows(false, 5, 1),
            vec![
                row(2, "Death takes its toll of 5 Gold"),
                row(50, "You have lost experience")
            ]
        );
        // No gold lost: row 2 takes row 1's place.
        assert_eq!(
            death_rows(false, 0, 2),
            vec![row(2, "You have lost experience")]
        );
        assert!(death_rows(false, 0, 0).is_empty());
        // Hardcore: only the valor message.
        assert_eq!(
            death_rows(true, 9, 2),
            vec![row(2, "Your deeds of valor will be remembered")]
        );
    }
}
