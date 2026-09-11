# Bluetooth Audio production installer

Build targets: xiaomi-band-10-pro-3.101.036, xiaomi-band-10-pro-3.101.043

Pack only main.lua and the .bin files in this directory. Requires the matching resident Canopus Supervisor.
Opening the watchface installs the signed module in the disabled state; it does not enable or bootstrap the framework.
Use an updated Supervisor with /canopus/install. External Lua uses ordinary IO only; no execute recovery or debug access is embedded.
The framework prepares /data/canopus/inbox before external installation. Signatures are verified against the Supervisor trust key during packaging; no private key is packaged.
