# nanomp3 (Canopus vendored copy)

Upstream: <https://github.com/robbie01/nanomp3> 0.1.1  
License: MIT OR Apache-2.0

The decoder implementation differs from upstream in two ways.

First, `mp3dec_scratch_t` is owned by `Decoder` and passed into
`mp3dec_decode_frame`. Upstream allocates this roughly 16 KiB scratch object as
a local variable for every decode call. Keeping it in caller-owned decoder
storage avoids overflowing the firmware Bluetooth callback stack and allows the
module to place the complete decoder workspace in explicit target-owned memory.

Second, `Decoder::decode` writes interleaved S16 samples instead of f32.
Synthesis uses minimp3's integer output scale (round to nearest,
saturating), which removes a separate f32-to-S16 pass from the playback path.
The synthesis functions are also marked `#[inline(never)]`, which lowers the
executed instruction count on Cortex-M33.

Input buffering remains the responsibility of BluetoothAudio. The public API is
otherwise compatible with nanomp3 0.1.1.
