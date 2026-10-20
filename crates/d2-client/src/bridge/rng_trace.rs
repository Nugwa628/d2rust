//! The RNG draw log's way out of the server (`rng-trace` feature): a link
//! whose server thread can drain its draw log (`d2_sim::debug::rng_trace`)
//! and read the known seeds implements [`RngTraceSource`]; the bridge
//! forwards the request, so the debug export reaches the game through the
//! bridge like everything else (step 5).

use d2_sim::debug::rng_trace::{Entry, Known};

/// A link whose server thread keeps an RNG draw log.
pub trait RngTraceSource {
    type Error;
    /// The draws logged since the last call, and the game and unit seeds
    /// now (read only).
    fn rng_trace_drain(&mut self) -> Result<(Vec<Entry>, Vec<Known>), Self::Error>;
}

impl<L: RngTraceSource> super::Bridge<L> {
    /// The server's draws since the last call (between two frames).
    pub fn rng_trace_drain(&mut self) -> Result<(Vec<Entry>, Vec<Known>), L::Error> {
        self.link.rng_trace_drain()
    }
}

/// The prediction wrapper forwards to the link it wraps.
impl<L: RngTraceSource> RngTraceSource for super::predict::PredictLink<L> {
    type Error = L::Error;
    fn rng_trace_drain(&mut self) -> Result<(Vec<Entry>, Vec<Known>), Self::Error> {
        self.inner_mut().rng_trace_drain()
    }
}
