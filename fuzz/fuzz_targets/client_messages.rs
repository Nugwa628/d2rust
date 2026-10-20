#![no_main]
// Share the server's synthetic wired cube/trade fixtures, including real handlers.
#[path = "../../crates/d2-server/tests/mutants_handlers_fx/mod.rs"]
mod fixture;
use d2_server::{adapters::ProtoSizes, buffers::ClientBuffers, dispatch, transport::ServerQueues};
use d2_sim::{rng::Seed, wiring::economy::GameFields};
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("client_messages", data);
    let _ = d2_proto::transport::client_size(data);
    let _ = d2_proto::transport::classify_client(data);
    let mut queues = ServerQueues::new();
    let _ = queues.send(&ProtoSizes, 0, data);
    let mut trade = fixture::TradeFx::new(0);
    let mut cube = fixture::CubeFx::new(
        1,
        GameFields::new(Seed::init_low(7), true),
        vec![fixture::ring_to_amulet()],
    );
    let mut out = ClientBuffers::new();
    out.add_client(0);
    let queued = queues.drain();
    for game in [&mut trade.sim, &mut cube.sim] {
        let _ = dispatch::dispatch(
            game,
            &ProtoSizes,
            &mut out,
            0,
            fixture::ALIVE,
            data,
            data.len(),
        );
        for message in &queued {
            let _ = dispatch::dispatch(
                game,
                &ProtoSizes,
                &mut out,
                0,
                fixture::ALIVE,
                &message.msg,
                message.size,
            );
        }
    }
});
