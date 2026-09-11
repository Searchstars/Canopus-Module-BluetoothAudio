# Bluetooth Audio production installer

Build targets: xiaomi-band-11-4.100.139

Pack only main.lua and the .bin files in this directory. Requires the matching resident Canopus Supervisor.
Opening the watchface installs the signed module in the disabled state; it does not enable or bootstrap the framework.
Use an updated Supervisor with /canopus/install. External Lua uses ordinary IO only; no execute recovery or debug access is embedded.
Band 11 adapters pass compiled ARM/firmware ABI tests; physical radio/audio/display remain NOT_PROBED.
The framework prepares /data/canopus/inbox before external installation. Signatures are verified against the Supervisor trust key during packaging; no private key is packaged.
