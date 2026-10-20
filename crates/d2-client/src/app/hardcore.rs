//! Hardcore and the saves of a death in the play preview.
//!
//! A hardcore character is marked by the save's status bit 0x4
//! (`d2s::status::HARDCORE`, or `play --hardcore` for `--new`). The game
//! then runs as a hardcore game: the server's client flag 4 is on
//! ([`super::single_player::LocalSeams::hardcore`]), so C→S 0x41 does not
//! respawn the dead player but drops the client with reason 3
//! (`handlers::player::resurrect`). Death is permanent: the character
//! save is written with the dead bit (0x8) as soon as the player dies, and
//! the next load refuses it (`d2s::read`: "dead hardcore
//! character"). The softcore character is saved at the same moment, so the
//! gold and experience the death took stay lost whichever way the window
//! is then closed.
//!
//! The save at the DD start is the spec's: the corpse then holds the cursor and
//! body items, and the save writes them in its corpse section (`save_full`).
//!
//! The death start (DT) saves nothing in the game; a window closed during the
//! death animation saves through the leave (`play::leave_on_exit`)
//! (r:l27-hc-save, 2026-10-14, L27).
//!
//! D2RUST's own (causes/run13-l27.tsv): the DD save above is asked by the app
//! ([`SaveHandle`]) and written by the server's character storage
//! (`save::FileStore`), because the server's own DD save is not wired yet; and
//! leaving a dead hardcore character closes the game (the game returns to
//! the character screen).

use bevy::app::AppExit;
use bevy::prelude::*;
use d2_formats::d2s;

use super::death::is_dead;
use super::save::SaveHandle;
use crate::bridge::BridgeResource;

/// Whether the save `bytes` belong to a hardcore character (status word
/// at 0x24, bit 0x4). A file shorter than the header is
/// not.
pub fn save_is_hardcore(bytes: &[u8]) -> bool {
    d2s::header_status(bytes).is_some_and(|s| s & d2s::status::HARDCORE != 0)
}

/// The running character is hardcore.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardcoreRun(pub bool);

pub use d2_server::game::character_save::mark_dead;

/// Saves the character once when the corpse lies (the local player in
/// mode 17, the DD start): the corpse section holds its items.
fn save_at_dd(
    bridge: Option<Res<BridgeResource>>,
    saver: Option<Res<SaveHandle>>,
    mut was: Local<bool>,
) {
    let dd = bridge.is_some_and(|b| {
        b.0.world()
            .local()
            .is_some_and(|p| p.mode == crate::bridge::modes::player_mode::DEAD)
    });
    if dd && !*was {
        if let Some(h) = &saver {
            match h.save() {
                Ok(()) => info!("play: saved the character at its corpse (DD)"),
                Err(e) => warn!("play: the corpse was NOT saved: {e}"),
            }
        }
    }
    *was = dd;
}

/// Hardcore: Esc on the dead player (the server drops the client, 0x41)
/// closes the game; the dead save is already written.
fn leave_dead(
    run: Res<HardcoreRun>,
    bridge: ResMut<BridgeResource>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    one_app: Option<Res<super::session::OneApp>>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    if run.0 && is_dead(bridge.0.world()) && keys.is_some_and(|k| k.just_pressed(KeyCode::Escape)) {
        println!("play: the hardcore character died; its save is marked dead");
        match one_app {
            Some(_) => commands.insert_resource(super::session::ToMenu),
            None => {
                exit.write(AppExit::Success);
            }
        }
    }
}

/// The systems of the wiring below, once per app (`app::session`).
pub fn register_hardcore(app: &mut App) {
    app.add_systems(
        Update,
        // The death
        // screen's state is written first; the save at death and the
        // hardcore leave read it the same frame.
        (
            save_at_dd.run_if(resource_exists::<HardcoreRun>),
            leave_dead
                .after(super::death::death_screen)
                .run_if(resource_exists::<BridgeResource>)
                .run_if(resource_exists::<HardcoreRun>),
        ),
    );
}

/// Installs the save at the DD start and, for a hardcore character, the leave.
pub fn add_hardcore(app: &mut impl super::session::Setup, hardcore: bool) {
    app.register("hardcore", register_hardcore);
    app.world_mut().insert_resource(HardcoreRun(hardcore));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_bit_is_read_from_the_header() {
        let mut b = vec![0u8; 0x30];
        assert!(!save_is_hardcore(&b));
        b[0x24] = d2s::status::HARDCORE as u8;
        assert!(save_is_hardcore(&b));
        assert!(!save_is_hardcore(&b[..0x20]));
    }

    #[test]
    fn only_a_hardcore_death_marks_the_save_dead() {
        let mut s = super::super::save::base_save(&super::super::single_player::Character::New);
        s.header.status = 0;
        mark_dead(&mut s, false, true);
        assert_eq!(s.header.status & d2s::status::DEAD, 0);
        mark_dead(&mut s, true, false);
        assert_eq!(s.header.status & d2s::status::DEAD, 0);
        mark_dead(&mut s, true, true);
        assert_eq!(s.header.status, d2s::status::HARDCORE | d2s::status::DEAD);
    }
}
