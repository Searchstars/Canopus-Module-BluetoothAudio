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
5. Open Canopus Manager and enable the module. Enabling `insmod`s the verified
   ELF; the module constructor runs `canopus_mod_prepare`, activation verifies
   identity, installs the `Headphones` launcher app + LVX pages, registers
   Bluetooth/SDP, and marks the module boot-resident, and the module is
   reported active in the Manager.

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
4. Open Canopus Manager, enable the **bluetooth_audio** module.

## On-device test scope (device gates)

Enabling the module installs the full headphone manager: `Headphones` launcher
app, overview + detail LVX pages, Bluetooth discovery/adapter callbacks,
L2CAP/AVDTP transport, SDP source, and the test tone. Each of these must be
verified on firmware `3.101.030` before the module counts as working:

- wrong-firmware rejection and clean failure before any registration;
- launcher visibility, icon/name, open/close/reopen, overview/detail
  navigation, page destruction, reboot behavior;
- UI-owner dispatcher under live scan/connect updates — no LVX call from
  Bluetooth/timer callbacks;
- repeated scans with multiple headsets, dedup, overflow indicator, cancel;
- bonded/unbonded selection, pairing prompts, AVDTP connect, failures/reconnect;
- five-second audible test tone with correct start/suspend and no leaked
  L2CAP/timer/SDP resources;
- RESIDENT irreversibility: unload requires a reboot.

Host tests and a verifier-clean ELF are not a substitute for these on-device
gates.
