//! Test suite of the server host: synthetic vectors and edge cases for the message
//! classifier, queues, buffers, dispatch, adapters and the tick driver, run against
//! the seam fakes in [`fakes`]. Each submodule covers one layer of `d2-server`.

mod adapters;
mod extension;
mod fakes;
mod game_thread;
mod gaps;
mod host;
mod hub;
mod messages;
mod packets;
mod tests_c2sima;
