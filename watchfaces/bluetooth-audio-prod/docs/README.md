# Bluetooth Audio production installers

Select a device folder, not this parent directory:

- `xiaomi-band-10-pro/`: firmware 3.101.036 and 3.101.043.
- `xiaomi-band-11/`: firmware 4.100.139; exact-target private runtime included.

The build selection controls which firmware versions are included in each device folder.
Pack that folder's single main.lua and all .bin files. The folder's build/ contains its ZIP and hash manifest; docs/ is not packed.
Update the framework Supervisor first: external installers use ordinary IO at /canopus/install.
No execute recovery or debug access is included. Older mixed-device outputs are retained only as build history.
