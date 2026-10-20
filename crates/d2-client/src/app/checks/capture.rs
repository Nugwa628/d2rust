//! Per-run tracing layer. Its dispatcher is inherited by the server thread,
//! so simultaneous repro workers never collect each other's errors.
use super::{v, Violation};
use bevy::log::{tracing, tracing_subscriber};
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::{registry::Registry, Layer};

#[derive(Clone)]
pub struct Capture {
    events: Arc<Mutex<Captured>>,
    parent: tracing::Dispatch,
}

#[derive(Default)]
struct Captured {
    violations: Vec<Violation>,
    seen: std::collections::BTreeSet<String>,
}

impl Default for Capture {
    fn default() -> Self {
        Self {
            events: Default::default(),
            parent: tracing::dispatcher::get_default(Clone::clone),
        }
    }
}

impl Capture {
    pub fn dispatch(&self) -> tracing::Dispatch {
        tracing::Dispatch::new(tracing_subscriber::registry().with(self.clone()))
    }

    pub fn take(&self) -> Vec<Violation> {
        std::mem::take(&mut self.events.lock().expect("check capture").violations)
    }
}

#[derive(Default)]
struct Fields {
    detail: String,
    subjects: std::collections::BTreeMap<String, String>,
}
impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        let _ = write!(self.detail, "{}={value:?} ", field.name());
        if matches!(field.name(), "unit" | "guid" | "asset") {
            self.subjects
                .insert(field.name().into(), format!("{value:?}"));
        }
    }
}

impl Layer<Registry> for Capture {
    fn enabled(&self, metadata: &tracing::Metadata<'_>, _: Context<'_, Registry>) -> bool {
        *metadata.level() == tracing::Level::ERROR
            || matches!(metadata.target(), "d2_repro_panic" | "d2_asset_fallback")
            || self.parent.enabled(metadata)
    }

    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, Registry>) {
        let metadata = event.metadata();
        let kind = if metadata.target() == "d2_repro_panic" {
            Some("panic")
        } else if metadata.target() == "d2_asset_fallback" {
            Some("missing-asset")
        } else if *metadata.level() == tracing::Level::ERROR {
            Some("error")
        } else {
            None
        };
        if let Some(kind) = kind {
            let mut fields = Fields::default();
            event.record(&mut fields);
            // Callsite identity, rather than message values or ticks, groups
            // repeat occurrences even when the error includes a frame number.
            let sig = format!(
                "{kind}:{}:{}:{:?}",
                metadata.target(),
                metadata.line().unwrap_or(0),
                fields.subjects
            );
            let mut captured = self.events.lock().expect("check capture");
            if captured.seen.insert(sig.clone()) {
                captured.violations.push(v(kind, sig, fields.detail));
            }
        }
        if self.parent.enabled(metadata) {
            self.parent.event(event);
        }
    }
}

/// The same panic payload is used by the client and server guards.
pub fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "non-string panic".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_and_asset_fallbacks_are_captured_but_warnings_are_quiet() {
        let capture = Capture::default();
        tracing::dispatcher::with_default(&capture.dispatch(), || {
            tracing::warn!("ordinary warning");
            assert!(capture.take().is_empty());
            tracing::error!(unit = 7, "broken state");
            tracing::warn!(target:"d2_asset_fallback",asset="synthetic.cof","missing asset");
        });
        let events = capture.take();
        assert_eq!(
            events.iter().map(|v| v.kind).collect::<Vec<_>>(),
            ["error", "missing-asset"]
        );
        assert!(events[0].detail.contains("unit=7"));
    }

    #[test]
    fn diagnostic_signatures_keep_units_and_ignore_ticks() {
        let capture = Capture::default();
        tracing::dispatcher::with_default(&capture.dispatch(), || {
            for (tick, unit) in [(1, 7), (2, 8), (3, 7)] {
                tracing::error!(tick, unit, "broken unit");
            }
        });
        let events = capture.take();
        assert_eq!(events.len(), 2);
        assert_ne!(events[0].signature(), events[1].signature());
        assert!(!events[0].signature().contains("tick"));
    }

    #[test]
    fn server_thread_inherits_capture() {
        use crate::app::server_thread::ThreadLink;
        use crate::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
        struct Link;
        impl ServerLink for Link {
            fn protocol_version(&self) -> u32 {
                1
            }
            fn tick_rate(&self) -> d2_sim::tick::TickRate {
                d2_sim::tick::TickRate::BASE
            }
            fn send(&mut self, _: SendQueue, _: &[u8]) -> Result<Sent, LinkError> {
                Ok(Sent::Queued)
            }
            fn pump(&mut self) -> Result<Pumped, LinkError> {
                Ok(Pumped::default())
            }
            fn receive(&mut self) -> Vec<Vec<u8>> {
                Vec::new()
            }
        }
        let capture = Capture::default();
        tracing::dispatcher::with_default(&capture.dispatch(), || {
            let mut link = ThreadLink::spawn(|| Ok::<_, String>(Link)).unwrap();
            link.with(|_| tracing::error!("server error")).unwrap();
            assert!(link.with(|_| panic!("server panic")).is_err());
            drop(link);
        });
        assert_eq!(
            capture.take().iter().map(|v| v.kind).collect::<Vec<_>>(),
            ["error", "panic"]
        );
    }
}
