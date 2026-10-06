# Bluetooth Audio production installer

Build targets: xiaomi-band-10-pro-3.101.036, xiaomi-band-10-pro-3.101.043

Pack only main.lua and the .bin files in this directory. Requires the matching resident Canopus Supervisor.
Opening the watchface installs the signed module in the disabled state; it does not enable or bootstrap the framework.
Uses /dev/canopus like the earlier Band 10 Pro installers, so any resident Supervisor works. Identity comes from /etc/build.prop (ro.build.customer_version), with getprop as fallback; the installer creates /data/canopus/inbox with mkdir.
Signatures are verified against the Supervisor trust key during packaging; no private key is packaged.
