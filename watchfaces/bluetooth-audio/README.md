# Bluetooth audio module installer watchface

A one-shot installer watchface that installs the Rust Bluetooth audio module
through the resident Canopus supervisor (`/dev/canopus`), exactly like
`watchfaces/canopus_hello` in the Canopus framework. Opening the watchface is
the only action required.

Runtime flow (fail-closed):

1. Validate the packaged receipt shape and ELF bounds.
2. Copy `receipt.bin` and `module.bin` to `/data/canopus/inbox/`.
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

- `CANOPUS_ROOT=/path/to/Canopus` — framework root (default `/Volumes/EXT0/Canopus`).
- `MODULE_INSTALL_KEY=/path/to/key.pem` — signer (default
  `<CANOPUS_ROOT>/.canopus-local/module-installer-ed25519.pem`).

`module.bin` and `receipt.bin` are build artifacts and are not committed.

## Install on device

1. Build the watchface payloads (above).
2. Install this watchface on the Band as a normal watchface.
3. Open it once. Expected result: `Installed — disabled by default.`
4. Enable the module in Canopus Manager, reboot, press **LOAD**, then press
   **INSTALL** in the Canopus installer. LOAD restores and automatically activates
   the backend; INSTALL runs in miwear, reinstalls Manager idempotently, and
   publishes the `Headphones` native app. Return to Manager to verify
   BOOT_RESIDENT or inspect the retained module error. Packages produced by older
   lifecycle-0 builds must be removed before this version is installed; do not
   overwrite only the inbox ELF.

The unsafe path is not used: `app_install` never runs from a Manager click or the
backend Activate callback. Publication is an ABI 1.1 callback invoked only by the
miwear-owned installer transaction. A publication failure is persisted for later
Manager display.

## On-device test scope (device gates)

Activation installs only the Bluetooth backend: discovery/adapter callbacks,
L2CAP/AVDTP transport, SDP source, and the test tone. Each must be verified on
firmware `3.101.030` before the module counts as working:

- wrong-firmware rejection and clean failure before any registration;
- automatic boot activation returns promptly without Manager lag, corruption,
  or reboot;
- repeated scans with multiple headsets, dedup, overflow indicator, cancel;
- bonded/unbonded selection, pairing prompts, AVDTP connect, failures/reconnect;
- five-second audible test tone with correct start/suspend and no leaked
  L2CAP/timer/SDP resources;
- RESIDENT irreversibility: unload requires a reboot.

Host tests and a verifier-clean ELF are not a substitute for these on-device
gates.
