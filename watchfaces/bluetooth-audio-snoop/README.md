# BluetoothAudio HCI Snoop Exporter

Install this directory as a normal watchface. It controls the firmware stock HCI
btsnoop recorder; it does **not** install, activate, or modify the BluetoothAudio
module.

## One-run capture flow

1. Open the watchface and press **Enable HCI snoop**.
2. Run one BluetoothAudio pairing/connection attempt.
3. Return immediately to this watchface and press **Close + Export snoop**.
4. The watchface disables the stock recorder, waits one second, and copies the
   **entire** source directory:

   ```text
   /data/misc/bt/snoop
   ```

   to:

   ```text
   /data/offlinelog/snoop
   ```

5. Export the device offline logs before another Bluetooth attempt, then reboot
   before making another capture.

The source is copied recursively. The exporter replaces only the destination
`/data/offlinelog/snoop`; it never deletes or changes `/data/misc/bt/snoop`.

The firmware btserver observes `persist.bluetooth.log.changed`, so enable/disable
uses the proven property sequence:

```text
setprop persist.bluetooth.log.snoop_enable 0|1
setprop persist.bluetooth.log.changed 0
setprop persist.bluetooth.log.changed 8
```
