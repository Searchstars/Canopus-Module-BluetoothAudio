# Bluetooth Audio production installers

Select a device folder, not this parent directory:

- `xiaomi-band-10-pro/`: xiaomi-band-10-pro-3.101.036, xiaomi-band-10-pro-3.101.043.
- `xiaomi-band-11/`: xiaomi-band-11-4.100.139, xiaomi-band-11-4.100.155.

The build selection controls which firmware versions are included in each device folder.
Pack that folder's single main.lua and all .bin files. The folder's build/ contains its ZIP and hash manifest; docs/ is not packed.
Band 11 installers use ordinary IO at /canopus/install: update the framework Supervisor first. Band 10 Pro installers keep the /dev/canopus flow and work with any resident Supervisor.
No execute recovery or debug access is included. Older mixed-device outputs are retained only as build history.
