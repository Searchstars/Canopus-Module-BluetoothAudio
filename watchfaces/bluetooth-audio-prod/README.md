# Bluetooth audio production installer

This watchface is the production, multi-target installer for the Bluetooth audio
module. One package contains separately built and signed payloads for:

- Xiaomi Band 10 Pro `3.101.030`
- Xiaomi Band 10 Pro `3.101.036`

At runtime `main.lua` reads `ro.build.version` and selects the module/receipt pair
whose full target ID matches that firmware. Unknown versions, malformed property
output, missing resources, and mismatched receipts fail before either inbox file
is written or an INSTALL request is sent. There is no fallback target.

## Build

From the repository root:

```sh
scripts/build-install-watchface-prod.sh
```

Overrides:

- `CANOPUS_ROOT=/path/to/Canopus` — framework root (default `../Canopus`).
- `MODULE_INSTALL_KEY=/path/to/key.pem` — receipt signer (default
  `<CANOPUS_ROOT>/.canopus-local/module-installer-ed25519.pem`).

The command builds, verifies, and signs both targets, then stages these generated
resources beside `main.lua`:

```text
bluetooth-audio-xiaomi-band-10-pro-3.101.030.bin
bluetooth-audio-xiaomi-band-10-pro-3.101.030.cmi.bin
bluetooth-audio-xiaomi-band-10-pro-3.101.036.bin
bluetooth-audio-xiaomi-band-10-pro-3.101.036.cmi.bin
```

The `.cmi.bin` suffix is intentional: the watchface packager accepts the
resource as a binary file. Lua reads it and writes the receipt to the device-side
`/data/canopus/inbox/bluetooth_audio.cmi` path. Each CMI1 receipt binds its own
exact target ID, firmware SHA-256, artifact size, and module SHA-256. The generated
payloads are ignored by Git.

Both this directory and the development installer carry the same
`appicon_headphones.bin` (117×117 LVGL v9 ARGB8888 with alpha, 54,768 bytes).
Lua stages it before INSTALL at `/data/canopus/appicon_headphones.bin`; the
native Headphones app uses that path when it registers its Launcher entry. To
regenerate the binary from `watchfaces/appicon_headphones.png`, use:

```sh
python3 /Volumes/EXT0/LVGLImage.py \
  --ofmt BIN --cf ARGB8888 --compress NONE --align 1 \
  -o watchfaces/bluetooth-audio watchfaces/appicon_headphones.png
cp watchfaces/bluetooth-audio/appicon_headphones.bin \
   watchfaces/bluetooth-audio-prod/appicon_headphones.bin
```

If an older module was already installed before this icon was added, open the
installer once to restage the icon before rerunning native app registration.

## Production differences

This package contains no test audio and never stages the development MP3 path.
The module is built with the non-default `production` feature, so the Headphones
detail page omits **Play test tone**, **Play long MP3**, and
**Decode long MP3 only**. The normal scan, pair, connect, refresh, disconnect,
and status functionality remains available. The development installer in
`../bluetooth-audio` retains all diagnostics and its audio fixture.

## Install

The device must already run the Canopus Supervisor and Manager. Install this
directory as a normal watchface and open it once. A supported firmware stages its
matching signed pair and sends the bounded CPC2 INSTALL request; the module is
installed disabled. Review and enable it in Canopus Manager, then reboot as
required by the resident lifecycle.

If the page reports `Firmware version not supported`, do not copy or rename a
payload from another firmware. Build and add a newly verified target pair first.
