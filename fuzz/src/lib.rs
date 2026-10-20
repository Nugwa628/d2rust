//! Shared helper for the fuzz targets.
//!
//! On Windows a Rust panic aborts through `__fastfail`, which libFuzzer's
//! crash handler does not see, so no `crash-*` file is written. [`arm`]
//! remembers the current input and installs a panic hook that writes it to
//! `artifacts/<target>/crash-<hash>` before aborting.

use std::cell::RefCell;
use std::sync::Once;

thread_local! {
    static CURRENT: RefCell<(String, Vec<u8>)> = const { RefCell::new((String::new(), Vec::new())) };
}

static HOOK: Once = Once::new();

/// Records `data` as the input of the current execution of `target`.
pub fn arm(target: &str, data: &[u8]) {
    HOOK.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            CURRENT.with(|c| {
                if let Ok(c) = c.try_borrow() {
                    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
                    for b in &c.1 {
                        h = (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3);
                    }
                    let dir = format!("artifacts/{}", c.0);
                    let _ = std::fs::create_dir_all(&dir);
                    let path = format!("{dir}/crash-panic-{h:016x}");
                    let _ = std::fs::write(&path, &c.1);
                    eprintln!("INPUT WRITTEN: {path} ({} bytes)", c.1.len());
                }
            });
            default(info);
            std::process::abort();
        }));
    });
    CURRENT.with(|c| {
        let mut c = c.borrow_mut();
        if c.0 != target {
            c.0 = target.to_owned();
        }
        c.1.clear();
        c.1.extend_from_slice(data);
    });
}
