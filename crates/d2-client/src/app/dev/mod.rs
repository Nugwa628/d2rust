// D2RUST's own: dev tools.
//! In-game developer tools for playtests: when they are on, the
//! name catalog, and the install of the server end ([`server`]) and the
//! overlay ([`overlay`]). Only `play::run` (the interactive window)
//! installs them; every check runner and test never does.

pub mod info;
pub mod overlay;
pub mod panel;
pub mod server;
pub mod slots;

use std::sync::{Arc, Mutex};

use super::server_thread::ThreadStopped;
use super::single_player::LiveData;
use super::strings::TableStrings;
use server::{DevCatalog, DevChannel, DevQueue, DevServer};

/// The environment variable that turns the tools off (`0`, `off`,
/// `false`).
pub const ENV: &str = "D2RUST_DEV";

/// Whether `D2RUST_DEV`'s value turns the tools off.
pub fn env_off(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        let v = v.trim();
        v == "0" || v.eq_ignore_ascii_case("off") || v.eq_ignore_ascii_case("false")
    })
}

/// On unless `--no-dev`, `D2RUST_DEV` off, or a check run.
pub fn enabled(no_dev_flag: bool, env: Option<&str>, check_run: bool) -> bool {
    !no_dev_flag && !env_off(env) && !check_run
}

/// The names of the user's tables.
pub fn catalog(live: &LiveData, strings: &TableStrings) -> DevCatalog {
    DevCatalog::from_live(live, |id| String::from_utf16_lossy(&strings.by_id(id)))
}

/// Installs the server end on the game's server thread (with the pokes
/// and sends of `--poke` / `--send`, which never run with the tools on:
/// Rule 1) and returns the overlay's end of the channel.
pub fn install_server<C: d2_server::seams::Clock + Send + 'static>(
    link: &mut super::server_thread::ThreadLink<super::single_player::Link<C>>,
    catalog: DevCatalog,
    start: Option<slots::DevStart>,
) -> Result<DevChannel, ThreadStopped> {
    let chan: DevChannel = Arc::new(Mutex::new(DevQueue::default()));
    let server = DevServer::new(catalog).with_start(start);
    link.set_before_pump(server::hook(server, chan.clone()))?;
    Ok(chan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_by_default_off_by_flag_env_or_check_run() {
        assert!(enabled(false, None, false));
        assert!(enabled(false, Some("1"), false));
        assert!(!enabled(true, None, false));
        assert!(!enabled(false, Some("0"), false));
        assert!(!enabled(false, Some("OFF"), false));
        assert!(!enabled(false, Some(" false "), false));
        assert!(!enabled(false, None, true));
    }
}
