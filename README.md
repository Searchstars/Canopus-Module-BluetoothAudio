# Canopus Bluetooth Audio

Exact-target, allocation-free Rust headphone manager module for Xiaomi Band 10 Pro firmware `3.101.030`.

## Status

The full module is implemented and builds to a verifier-clean, size-budgeted ELF:

- portable core (`bluetooth-audio-core`): discovery table (12-entry, dedup), pair/connect controller, AVDTP source, SBC/RTP five-second tone packetizer, bounded Canopus semantic UI — host-tested.
- exact-target backend (`bluetooth-audio-device`, `device` feature): identity guard, native app + launcher install, two LVX pages (overview + detail), UI-owner-thread dispatcher, Bluetooth adapter callbacks, L2CAP signaling/media, SDP source registration, media tone timer, and RESIDENT irreversible lifecycle.
- every absolute firmware address and ABI record lives in the framework's `canopus-target-private` crate; the module hardcodes no address, and the required firmware calls are intentionally not added to Canopus's generated public bindings.

Current artifact: `scripts/build-device.sh` is fully green (fmt, workspace + cross clippy, host tests, lean nightly cross-build, `ld.lld -r`, `.llvmbc`/debug strip, Canopus verifier PASS: 61 sections, 0 undefined, 727 relocs, 1 ctor, 1 dtor). Loaded image is **31,906 bytes** against the firmware's 65,536-byte limit, and the file (57,372 B) fits the 128 KiB CMI1 receipt bound. `scripts/build-install-watchface.sh` stages `watchfaces/bluetooth-audio/` with the signed receipt and ELF.

What remains is **device-gated**: launcher visibility, LVX navigation, the UI-owner dispatcher under live scan/connect updates, and real pairing/connect/test-tone on firmware `3.101.030`. Host success and a verifier-clean ELF are not a substitute for those on-device gates; until they pass, the module must not be described as an end-to-end working headphone manager.

## Build

```sh
cargo test --workspace
scripts/build-device.sh
```

The cross-build runs on the **nightly** toolchain (`cargo +nightly`), which must be installed (`rustup toolchain install nightly`). The cross-binary is kept under the 64 KiB loader limit with two levers:

- `-C symbol-mangling-version=hashed` (needs `-Z unstable-options`) shrinks the long Rust symbol/section names that dominate `.symtab`/`.strtab`;
- `-Z function-sections=no` merges the ~890 per-function sections, collapsing `.shstrtab` and the section headers.

`RUSTFLAGS` replaces the `[target.*]` flags in `.cargo/config.toml`, so the scripts repeat `-C panic=abort -C target-cpu=cortex-m33`. After `ld.lld -r`, `rust-objcopy --remove-section=.llvmbc --strip-debug` drops the unconsumed thin-LTO bitcode and debug metadata. Override the toolchain with `NIGHTLY_CARGO`, the objcopy binary with `RUST_OBJCOPY`, and the framework root with `CANOPUS_ROOT`.

Dependencies are local paths into `/Volumes/EXT0/Canopus/sdk/rust`.

## Install on device (watchface)

The module is packaged as a one-shot installer watchface using the Canopus
“install watchface” concept (see `watchfaces/canopus_hello` in the framework).

```sh
scripts/build-install-watchface.sh
```

This cross-builds the module, runs the Canopus ELF verifier, signs the CMI1
receipt with the local dev key, smoke-tests the Lua installer, and stages the
payloads into `watchfaces/bluetooth-audio/`:

```text
watchfaces/bluetooth-audio/
├── main.lua       one-shot installer (opening the watchface installs the module)
├── module.bin     verified zero-import ELF32 ET_REL (gitignored)
└── receipt.bin    signed CMI1 receipt for token `bluetooth_audio` (gitignored)
```

Device steps (after the Canopus supervisor + Manager are installed via the
framework's `canopus-installer` watchface):

1. Install `watchfaces/bluetooth-audio` as a normal watchface.
2. Open it once. The supervisor verifies the Ed25519 signature, exact target
   and firmware identity, artifact size, and SHA-256, then registers the
   module as installed and disabled.
3. Open Canopus Manager and enable the `bluetooth_audio` module.

Enabling loads the identity-guarded lifecycle, installs the `Headphones`
launcher app with overview/detail LVX pages, registers Bluetooth/SDP, and marks
the module boot-resident (unload then requires a reboot). Each of these steps —
launcher visibility, page navigation, the UI-owner dispatcher under live scan
updates, and pairing/connect/test-tone — is a device gate that must be verified
on firmware `3.101.030` before the module is considered end-to-end working.

## Verification

```sh
scripts/build-device.sh      # fmt, clippy, tests, cross-build, link, verifier, size
scripts/build-install-watchface.sh   # + receipt signing + Lua smoke + staging
```
