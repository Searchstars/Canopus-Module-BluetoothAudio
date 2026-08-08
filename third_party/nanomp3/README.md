# nanomp3 (Canopus vendored copy)

Upstream: <https://github.com/robbie01/nanomp3> 0.1.1  
License: MIT OR Apache-2.0

The decoder implementation is unchanged except that `mp3dec_scratch_t` is owned
by `Decoder` and passed into `mp3dec_decode_frame`. Upstream allocates this
roughly 16 KiB scratch object as a local variable for every decode call. Keeping
it in caller-owned decoder storage avoids overflowing the firmware Bluetooth
callback stack and allows the module to place the complete decoder workspace in
explicit target-owned memory.

Input buffering remains the responsibility of BluetoothAudio. The public API is
otherwise compatible with nanomp3 0.1.1.
