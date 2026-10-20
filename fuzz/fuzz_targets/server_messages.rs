#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("server_messages", data);
    let _ = d2_proto::transport::server_size(data);
    let _ = d2_proto::s2c::parse(data);
    if let Ok(messages) = d2_proto::transport::split_server_buffer(data) {
        for message in messages.messages {
            let _ = d2_proto::s2c::parse(message);
        }
    }
});
