# Himeko local patch for msedge-tts 0.4.0

This directory is copied from the exact crates.io `msedge-tts` 0.4.0 package resolved by Himeko's pre-patch `Cargo.lock`.

Local production change: reject binary WebSocket frames shorter than the two-byte header-length field and reject declared header ranges that exceed the frame payload. This prevents malformed external frames from panicking or reaching an out-of-bounds audio slice.

The vendored unit tests retain both malformed-frame regressions and the abrupt-EOF liveness probe. No EOF-loop production change is applied because the exact transport probe passes: abrupt transport EOF terminates synthesis with an error within the test deadline.

The upstream `rodio` and `smol` dev-dependencies are removed from this vendored test manifest because Himeko does not use upstream audio-playback examples or smol-runtime tests. Production dependency features remain unchanged (`tokio-runtime`, no default features).
