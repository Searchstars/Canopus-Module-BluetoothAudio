# Firmware 3.101.030 Bluetooth path parity

This module targets only `xiaomi-band-10-pro-3.101.030`. The working reference is
`/Volumes/EXT0/firmware_latest/native`, whose Phase 5/6 device run completed fresh
Classic pairing, two outbound L2CAP channels, AVDTP negotiation, and a five-second
SBC stream.

## Authoritative sequence

| Step | Working native implementation | Module implementation |
|---|---|---|
| Register adapter client | `bt_adapter_bridge.c:1765-1846`, slots 0/1/2/5/6/9 | `target/bluetooth.rs::register` |
| Start discovery | `0x0C398D61(adapter, 20)` | `DevicePlatform::start_discovery` |
| Stop discovery | `0x0C398D8D(adapter)`; wait for STOPPED callback | `stop_discovery` then `CORE_EVENT_DISCOVERY_STOPPED` |
| Aggregate bond query | `0x0C39F371(address)` | `query_bond` |
| Exact Classic record query | `0x0C39F9B1(address, 1)` | `query_bond` |
| Remove retained Classic record | `0x0C3A028D(address, 1)` | `prepare_fresh_bond` |
| Removal commit | matching bond callback `(transport=1, state=0)` | `CORE_EVENT_REMOVE_OK` |
| Make adapter bondable | `set_scan_mode(current_mode, 1)` | `submit_bond` |
| Install target-only policy filter | mirror 16 callbacks; replace slot 5 only | `install_core_pair_filter` |
| Create fresh Classic bond | `0x0C3A01A9(address, 1)` | `submit_bond` |
| Pair Request reply | `0x0C39988D(adapter, address, 1)` | `on_pair_request` |
| Numeric comparison reply | `0x0C3998C9(adapter, address, 1, 1)` | `on_pair_display` |
| Bond commit | matching callback `(transport=1, state=2)` | `CORE_EVENT_BOND_OK` |
| Signaling L2CAP | PSM `0x0019`, 68-byte request | `submit_connect`: local receive MTU `0x0400`, option bit 0 enabled |
| AVDTP | DISCOVER, GET ALL CAPABILITIES/fallback, SET CONFIGURATION, OPEN | `avdtp::Source` |
| Media L2CAP | second outbound PSM `0x0019` | `media_submit_connect`: same local receive MTU policy |
| Stream | START, RTP/SBC, SUSPEND | `TonePacketizer` and media timer |

## Host-mode Pair Request policy

The product Bluetooth client is bound to the phone companion and its stock Pair
Request callback at `0x0C6E1E25` rejects a different address. After bind, that
produces IO Capability Request Negative Reply (`0x0434`) with reason `0x18`.

The working reference and this module use the same narrow workaround:

1. Read the stock callback table at `0x2CD1F930`.
2. Copy all 16 words into resident memory.
3. Replace only callback slot 5.
4. Register the mirror through `0x0C398C25`.
5. Unregister the original handle through `0x0C398C8D`.
6. Store the mirror handle in the product callback-handle slot.
7. Suppress the original callback only for the selected target while a stock
   PairDevice transaction is pending.
8. Forward every unrelated request to the exact original callback.

The module does not change bind state, overwrite the companion address, forge a
remote security record, or submit raw pairing HCI commands. The stock PairDevice
operation remains responsible for Authentication Requested, IO Capability
exchange, numeric confirmation, link-key notification, authentication complete,
encryption, and the final BONDED callback.

## Recovered ABI details

- Adapter state ON: `4`.
- Classic transport: `1`.
- Exact device-record BONDED: `2`.
- Aggregate Classic/key-present state used by the reference: `3`.
- `create_bond` and `remove_bond`: zero means accepted/success.
- Pair Request and Pair Display public wrappers: zero means success.
- L2CAP connect at `0x0C7ED49D` returns the queue node: nonzero means accepted,
  zero means queue insertion failed.
- In the 68-byte connect request, the stock worker copies the local receive MTU
  from offset 52 only when option bit 0 at offset 54 is set. Both AVDTP channels
  now request MTU 1024, producing standard option bytes `01 02 00 04`; writing a
  value at offset 52 while leaving the option bitmap zero produces an empty
  Configuration Request.
- The live GAP transport vtable is rebuilt in writable DATA at
  `0x20138070..0x20138084` when Bluetooth powers on. Its send entry at
  `0x2013807C` is stock Thumb callback `0x0C7F9D11`; the HCI host stack invokes
  that entry with every complete outbound H4 packet, before the callback
  forwards through the lower transport function stored at `0x20137EF0`. The
  boot-resident compatibility wrapper compare-checks the vtable slot, filters
  only complete ACL/L2CAP signaling Information Responses. Extended Features
  `(type=2, result=0, mask=0x000001B8)` is changed to `0x000000B8`, and the
  exact Fixed Channels response `(type=3, result=0, mask=0x02)` is changed to
  Android's `0x82`; all other bits and packets are forwarded unchanged. The
  module reasserts the hook at adapter ON and immediately before
  discovery/bond/connect operations because the power-on path reconstructs the
  table. The previously
  tested `/dev/ttyBT0` callback at `0x200ED89C + 8` is not on this stock ACL
  transmit path and is intentionally not modified. The filter does not modify
  the incoming `0x7F` Configuration option.
- L2CAP callback events: 2 confirm, 3 complete, 4/5 informational, 6 disconnect,
  7 data, 8 flow telemetry.
- L2CAP completion fields: MTU at offset 72 and CID at offset 108.
- Signaling data fields: total at 0, offset at 2, CID at 4, payload at
  `packet + 4 + offset`.
- Firmware L2CAP callbacks return zero after recording internal failures. The
  Rust callbacks follow that convention so an internal parser/state error is not
  accidentally interpreted by the stock callback dispatcher as ownership or
  control flow.

## AVDTP and media constants

- Local SEID: 1.
- SBC: 44.1 kHz, stereo, 16 blocks, 8 subbands, Loudness, bitpool 53.
- SBC capability/config bytes: `0x22`, `0x15`, `0x35`, `0x35`.
- Frame length: 118 bytes; samples per frame: 128.
- Five frames per RTP packet where MTU permits.
- RTP payload type: 96.
- RTP SSRC: `0x42545036` (`BTP6`), matching the device-proven artifact.
- Full tone: 345 packets and 1,725 frames.
- Media timer event: 9; tag: `A2DPM`; one-shot flag: 1.

## Deliberate safety extensions

These do not replace firmware ordering or pairing behavior:

- Eight-second removal and 60-second pairing watchdogs prevent a permanently
  pending UI transaction when an authoritative callback never arrives.
- Critical callback payloads are moved to Bluetooth-owner queued work if the
  Rust core lock is temporarily busy; the original firmware callback never
  blocks.
- Media callback and timer generations reject stale work from a prior session.
- L2CAP/SDP queue insertion returns are checked instead of silently waiting for
  callbacks that cannot arrive.
- Discovery results carry a scan epoch when deferred, preventing a result from a
  previous scan from entering the next scan's table.
- UI updates run on a page-owned LVGL timer and retain the existing content root,
  rows, labels, and semantic address keys.

## Remaining device gates

Static analysis and host tests cannot prove controller/air behavior. Before this
path is considered device-verified, a run must observe in order:

1. discovery STOPPED before bond operations;
2. aggregate and exact bond queries;
3. target-only remove submission and Classic NONE callback;
4. Pair Request filter hit;
5. Pair Request and Pair Display callbacks;
6. link-key/authentication/encryption completion and Classic BONDED callback;
7. signaling CID/MTU and first DISCOVER;
8. SET CONFIGURATION and OPEN;
9. media CID/MTU and START;
10. 345 RTP/SBC packets followed by SUSPEND;
11. clean media and signaling disconnect callbacks;
12. a second complete fresh-pair run without a stale callback changing the new
    session.

A retained-bond fast reconnect option is intentionally absent until repeated
true-device runs demonstrate at least 95% reliability across supported peers.
