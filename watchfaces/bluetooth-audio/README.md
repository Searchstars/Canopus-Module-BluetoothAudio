# Bluetooth audio module installer watchface

A one-shot installer watchface that installs the Rust Bluetooth audio module
through the resident Canopus supervisor (`/dev/canopus`), exactly like
`watchfaces/canopus_hello` in the Canopus framework. Opening the watchface is
the only action required.

Runtime flow (fail-closed):

1. Validate the packaged receipt shape and ELF bounds, then derive an audio-only
   MP3 test resource by removing the 1.13 MiB ID3 artwork from
   `long_test_audio.bin` without transcoding its audio frames.
2. Copy `receipt.bin` and `module.bin` to `/data/canopus/inbox/`, and copy the
   derived stream to `/data/canopus/tmp_btaudio_module_long_audio_test.mp3`.
3. Send a bounded CPC2 `INSTALL` request containing only the `bluetooth_audio`
   token.
4. The supervisor verifies the CMI1 Ed25519 signature, exact target and
   firmware identity, artifact size, and SHA-256 digest, then registers the
   module as **installed and disabled**.
5. Open Canopus Manager and enable the module. After reboot and LOAD, the
   constructor prepares and self-registers the descriptor as READY; the
   supervisor immediately activates it and persists either BOOT_RESIDENT or the
   callback error for later Manager display. The subsequent Canopus installer
   **INSTALL** transaction publishes the module's `Headphones` app in miwear.

## Prerequisites on the device

The Band must already run the Canopus supervisor and native Manager. Install
the `watchfaces/canopus-installer` watchface from the Canopus framework first,
open it, press **LOAD** once, then **INSTALL** once (exactly as its README
describes). After that `/dev/canopus` exists and this installer watchface can
talk to it.

## Build the package payloads

```sh
scripts/build-install-watchface.sh
```

This cross-builds the module, runs the Canopus ELF verifier, signs the CMI1
receipt with the local dev key, and stages `module.bin` + `receipt.bin` here.
Overrides:

- `CANOPUS_ROOT=/path/to/Canopus` — framework root (default `../Canopus`).
- `MODULE_INSTALL_KEY=/path/to/key.pem` — signer (default
  `<CANOPUS_ROOT>/.canopus-local/module-installer-ed25519.pem`).

`module.bin` and `receipt.bin` are build artifacts and are not committed.

The watchface also carries `appicon_headphones.bin`, a 117×117 LVGL v9
ARGB8888 image with alpha (54,768 bytes). It is staged before INSTALL to
`/data/canopus/appicon_headphones.bin`, which is the icon path used by the
native Headphones app. To regenerate it from the tracked source:

```sh
python3 /Volumes/EXT0/LVGLImage.py \
  --ofmt BIN --cf ARGB8888 --compress NONE --align 1 \
  -o watchfaces/bluetooth-audio watchfaces/appicon_headphones.png
cp watchfaces/bluetooth-audio/appicon_headphones.bin \
   watchfaces/bluetooth-audio-prod/appicon_headphones.bin
```

If an older module was already installed before this icon was added, open this
installer once to restage the icon before running the native app registration
stages again.

## Install on device

1. Build the watchface payloads (above).
2. Install this watchface on the Band as a normal watchface.
3. Open it once. Expected result: `Installed — disabled by default.`
4. Enable the module in Canopus Manager, reboot, and press **LOAD** in the
   Canopus installer. Then press **INSTALL** three times, allowing each callback
   to return before the next press: stage 0 registers Manager, stage 1 registers
   the Headphones app/pages, and stage 2 adds its Launcher entry. LOAD restores
   and automatically activates the backend. Return to Manager to verify
   BOOT_RESIDENT or inspect the retained module error. Packages produced by older
   lifecycle-0 builds must be removed before this version is installed; do not
   overwrite only the inbox ELF.

The unsafe path is not used: `app_install` never runs from a Manager click or the
backend Activate callback. ABI 1.2 gives app/page registration and Launcher
publication distinct miwear-owned transactions. A publication failure is
persisted for later Manager display.

## On-device test scope (device gates)

Activation installs the Bluetooth backend and audio endpoint: discovery/adapter callbacks,
L2CAP/AVDTP transport, SDP source, the test tone, and the packaged long-MP3 test. Each must be verified on
firmware `3.101.030` before the module counts as working:

- wrong-firmware rejection and clean failure before any registration;
- automatic boot activation returns promptly without Manager lag, corruption,
  or reboot;
- repeated scans with multiple headsets, dedup, overflow indicator, cancel, and automatic in-place page updates without scroll reset;
- for every selection: asynchronous scan-stop completion, separate aggregate/exact bond query, target-only local Classic bond removal when present, authoritative NONE confirmation, then a fresh stock bond;
- Pair Request/Pair Display acceptance, SSP/link-key/authentication/encryption, authoritative BONDED handoff, AVDTP signaling/media connect, timeout and reconnect behavior;
- five-second audible test tone and the separate packaged 96-second real-MP3 test,
  including 24→44.1-kHz resampling, correct drain/release, and repeat playback
  without leaked L2CAP/timer/file resources;
- RESIDENT irreversibility: unload requires a reboot.

Host tests and a verifier-clean ELF are not a substitute for these on-device
gates.
