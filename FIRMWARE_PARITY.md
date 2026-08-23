# Firmware parity reference

This document records the historical 3.101.030 Bluetooth path used while the
036 implementation was being established. 3.101.030 is no longer a supported
build target; active module builds are 3.101.036 and 3.101.043.

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
| Install transaction Pair Request filter | mirror 16 callbacks; replace slot 5 only | `install_core_pair_filter` |
| Create fresh Classic bond | `0x0C3A01A9(address, 1)` | `submit_bond` |
| Pair Request reply | `0x0C39988D(adapter, address, 1)` | `on_pair_request` |
| Numeric comparison reply | `0x0C3998C9(adapter, address, 1, 1)` | `on_pair_display` |
| Bond commit | matching callback `(transport=1, state=2)` | `CORE_EVENT_BOND_OK` |
| Signaling L2CAP | PSM `0x0019`, 68-byte request | `submit_connect`: local receive MTU `0x0400`, option bit 0 enabled |
| AVDTP | DISCOVER, GET ALL CAPABILITIES/fallback, SET CONFIGURATION, OPEN | `avdtp::Source` |
| Media L2CAP | second outbound PSM `0x0019` | `media_submit_connect`: same local receive MTU policy |
| Stream | START, RTP/SBC, SUSPEND | `TonePacketizer` or MP3 `StreamPacketizer`, software SBC, and owner timers |

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
- Three device A/B experiments changed the module's local receive MTU from an
  empty request to 1024, Extended Features from `0x000001B8` to `0x000000B8`,
  and Fixed Channels from `0x02` to Android's `0x82`. All three wire changes
  took effect and none stopped the peer from sending `7F 01 01`. The final
  candidate retains the MTU fix required by the stock connect ABI but leaves
  both outbound Information Responses stock, avoiding unnecessary global
  capability changes.
- The 036 GAP receive callback slot at `0x20137EA4` normally contains stock
  Thumb dispatcher `0x0C7D3E0D`. Exact 043 instead resolves firmware-owned
  `owner = *(void **)0x2013BDB4`, `state = *(void **)owner`, and the separately
  allocated callback cell at `*(void ***)(state + 0x28)`; its stock callable is
  `0x0C7EC47D`. Target-private backends own only this exact writable seam, the
  stock/replacement compare guard, and power-transition ownership. They do not
  own mHDT policy or packet parsing.
- The portable compatibility layer requires that seam on every runtime-capable
  target and fails closed with `ERR_HCI_POLICY` when it is unavailable. It learns
  the local dynamic CID directly from a successful inbound L2CAP Connection
  Response, so the following peer Configuration Request need not wait for a
  firmware connection-confirm callback. For the matching signaling or media
  transaction only, it removes exact BES mHDT option `7F 01 01`, repairs the HCI
  ACL, L2CAP, signaling-command, and forwarded packet lengths, then lets the
  unmodified stock parser process the remaining MTU option and complete its
  normal channel transition. It neither rewrites unrelated dynamic channels,
  advertises local mHDT support, nor enables mHDT controller mode. The observed
  packet is a complete PB=2 ACL start packet with a 19-byte ACL payload;
  continuation fragments are forwarded untouched and are not misparsed as L2CAP
  headers.
- The module clears its CID hints for each new signaling/media connect and
  reasserts the receive hook at adapter ON and before discovery, bond, or connect
  because the power-on path reconstructs the dispatcher. Diagnostic bit `0x80`
  records successful hook installation independently of bit `0x40`, which
  records an actual mHDT rewrite. The previously tested `/dev/ttyBT0` callback at
  `0x200ED89C + 8` and GAP send slot `0x2013807C` remain stock.
- L2CAP callback events: 2 confirm, 3 complete, 4/5 informational, 6 disconnect,
  7 data, 8 flow telemetry.
- L2CAP completion fields: MTU at offset 72 and CID at offset 108.
- Signaling data fields: total at 0, offset at 2, CID at 4, payload at
  `packet + 4 + offset`.
- Firmware L2CAP callbacks return zero after recording internal failures. The
  Rust callbacks follow that convention so an internal parser/state error is not
  accidentally interpreted by the stock callback dispatcher as ownership or
  control flow.

## Stock profile-registration diff

The stock product `profile_init` at `0x0C53FDB4` is not an in-process Audio
Source registration path. It registers the product **headset/A2DP Sink** client
through `0x0C398ABC`, then registers HFP AG. `0x0C398ABC` allocates a Bluetooth
socket callback handle and sends operation 12 through `0x0C395C78`; the latter
serializes a fixed 708-byte IPC envelope. Its callback at `0x0C542421` is named
`headset_connection_state_callback`, and after A2DP reaches CONNECTED it starts
`headset_ag_connect`. Reusing that descriptor would register the module as the
product Sink/headset client, not create an Audio Source endpoint.

The lower snoop classifier at `0x0C3AC5FC` recognizes AVDTP signaling solely
from an L2CAP Connection Request whose PSM is 25 (`0x0019`) and records its CID
pair. It does not consult the product callback descriptor, a service ID,
authorization mask, or an AVDT control block. The device-proven custom Source
implementation likewise registers its local Source SDP record and opens raw
PSM `0x0019` channels; no separate Source/security registration call exists in
that path.

The remaining concrete Android/Vela distinction is controller context, not an
invisible PSM flag. Android's successful capture reports local controller
manufacturer `0x001D` (Qualcomm), while the REDMI peer reports manufacturer
`0x02B0`; this firmware runs on the BES platform. BES mHDT state explicitly
tracks controller features learned through vendor HCI separately from host
support learned through L2CAP Configuration. Its 8DH5 maximum payload is 2820,
exactly matching the peer's MTU `0x0B04` beside `7F 01 01`. This is why the
final compatibility candidate handles the parser/controller mismatch rather
than registering the product Sink IPC client.

Android also completes a targeted remote Audio Sink `0x110B`
ServiceSearchAttribute request for attributes `0x0001`, `0x0004`, and `0x0009`
before opening PSM `0x0019`. The recovered SDK exposes only local SDP server
registration, not a remote SDP client ABI. Reproducing that sequence would
require an additional raw PSM `0x0001` client, continuation parser, timeout, and
channel lifecycle. It remains a separate device A/B only if the exact mHDT
compatibility path fails; it is not mixed into the current candidate because it
does not explain the paired `2820 + 7F` selection.

## AVDTP and media constants

- The first REDMI run after mHDT filtering passed L2CAP Configuration, sent
  DISCOVER and GET ALL CAPABILITIES, then stopped on the accepted SBC capability
  `3F FF 02 27`. The old selector required `remote_min <= 53 <= remote_max`;
  `remote_max=0x27` made `choose_sbc` return `Unsupported`, which the transport
  surfaced as `-1104`, before SET CONFIGURATION could be emitted. Local SEID 1
  and remote Sink SEID 1 were both valid. The selector now intersects the peer
  range with resident bitpools 27..53, emits the negotiated range 27..39 in
  SET CONFIGURATION, and transmits the tone at bitpool 39 for this response.
- Local SEID: 1.
- SBC: 44.1 kHz, stereo, 16 blocks, 8 subbands, Loudness. The source selects
  the highest peer-supported bitpool in its resident 27..53 encoder-frame range;
  REDMI's `02..27` capability therefore selects bitpool 39 instead of failing
  the previous hard requirement for 53.
- SBC capability bytes advertised locally: `0x22`, `0x15`, `27`, `53`.
- Frame length is `12 + 2 * bitpool`: 90 bytes at bitpool 39 and 118 bytes at
  bitpool 53; samples per frame remain 128.
- Five frames per RTP packet where MTU permits.
- RTP payload type: 96.
- RTP SSRC: `0x42545036` (`BTP6`), matching the device-proven artifact.
- Full tone: 345 packets and 1,725 frames.
- Media timer event: 9; tag: `A2DPM`; one-shot flag: 1.
- AVDTP Delay Report is retained in 100-microsecond units instead of merely
  acknowledged. At START the source queues a bounded presentation-buffer burst
  sized from that report (11 packets for REDMI's 150 ms report, with 150 ms as
  the fallback), then waits all but one packet interval before normal pacing.
  After the last packet it waits the reported presentation delay plus one final
  packet interval before marking that tone complete. The accepted AVDTP stream
  remains active, avoiding a peer-specific SUSPEND/START round trip between
  playback chunks while preserving the five-second RTP timeline.
- A second Play action installs a fresh packetizer on the live stream. If the
  peer closed only the media channel while idle, the action reconnects PSM
  `0x0019` and continues after its completion callback rather than returning
  `-1201`. A remote-initiated SUSPEND still transitions the source back to OPEN
  and the next Play uses the normal START command.

## Third-party MP3 stream candidate

The target-independent `/dev/canopus_audio` ABI now feeds a bounded owner-thread
pipeline: 16 KiB compressed ring, incremental `nanomp3`, stereo S16 with 0..100
software gain, bounded phase-carrying 24/48→44.1-kHz linear resampling (the
packaged real-audio fixture exercises 24 kHz and a host fixture exercises 48 kHz;
44.1-kHz input remains direct), the vendored BlueZ
SBC encoder at the negotiated bitpool, and
variable-length RTP packets paced by a generation-checked timer. START creates a
fresh playback generation; PAUSE/RESUME retain decoder and encoder state; STOP,
FLUSH, close, and DRAIN invalidate stale work with distinct semantics.

Large decoder, PCM, SBC, input-ring, and mutable core storage is allocated from
target-owned RAM rather than the module image or a firmware callback stack. The
75 KiB loaded artifact fits target-pack revision 3's 80 KiB project budget. A
build-generated constructor fixup encodes only verifier-reported codec-table
words that accidentally resemble XIP addresses, then restores them in modlib RAM
before Rust executes; the final ELF remains zero-import and verifier-clean.
Host tests cover fragmented MP3 writes, in-place workspace initialization, state
generations, volume conversion, RTP progression, and decoding/resampling the exact
packaged 24-kHz long-audio resource. The build strips only its 1.13 MiB ID3
artwork (without transcoding audio) before the installer copies the audio-only
stream to `/data/canopus/tmp_btaudio_module_long_audio_test.mp3`; the Headphones detail page
exposes a separate **Play long MP3** action that feeds it incrementally through
the same exclusive input state machine and owner-thread pipeline. CPU cost, heap headroom,
startup prebuffering, pause/drain behavior, and sustained playback remain device
gates; this candidate must not yet be described as device-stable.

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

## Exact 3.101.043 retest status

The reported `Bond 3/2 0F -1105` proves Classic bonding completed, but the old
build recorded no `0x40` rewrite before the later `ERR_REMOTE`. The compact line
cannot distinguish an AVDTP Reject from a non-clean signaling disconnect. The
host-fixed exact 043 artifact is:

```text
build/xiaomi-band-10-pro-3.101.043/bluetooth-audio.elf
SHA-256 5844f0fc90aa3aeab9761349ed767f90bbf31f6b17444886c3dfc2506b320b62
```

Its verifier result is PASS with 188 sections, zero undefined symbols, 1,377
relocations, one constructor, one destructor, and 119,025 loaded bytes. This is
host evidence only. A clean reboot is required before a retest so an older
resident module or callback replacement cannot survive in the observation.
The new compact diagnostics mean:

- `Bond 3/2 8F`: hook installed, no mHDT rewrite observed;
- `Bond 3/2 CF`: hook installed and exact mHDT option rewritten;
- `-1109`: exact raw-H4 hook installation failed closed;
- `Bond 3/2 CF -1105`: execution passed mHDT compatibility and the remaining
  failure needs a separate AVDTP Reject versus disconnect-stage diagnosis.

No 043 device-success conclusion is recorded until that retest exists.

## Remaining device gates

Static analysis and host tests cannot prove controller/air behavior. Before this
path is considered device-verified, a run must observe in order:

1. discovery STOPPED before bond operations;
2. aggregate and exact bond queries;
3. target-only remove submission and Classic NONE callback;
4. Pair Request filter hit;
5. Pair Request and Pair Display callbacks;
6. link-key/authentication/encryption completion and Classic BONDED callback;
7. raw-H4 compatibility hook installed (`0x80`), successful wire Connection
   Response CID learning, and exact mHDT rewrite when present (`0x40`);
8. signaling CID/MTU and first DISCOVER;
9. SET CONFIGURATION and OPEN;
10. media CID/MTU and START;
11. 345 RTP/SBC packets, presentation drain, and reusable active stream;
12. clean media and signaling disconnect callbacks;
13. a second complete fresh-pair run without a stale callback changing the new
    session.

A retained-bond fast reconnect option is intentionally absent until repeated
true-device runs demonstrate at least 95% reliability across supported peers.

## REDMI Buds 8 Pro validation status

The device run of module SHA-256 `c43f269b…cfa98e23` confirmed fresh pairing,
the `mhdt-fixed` receive path, successful signaling Configuration, DISCOVER,
GET ALL CAPABILITIES, bitpool-39 negotiation, SET CONFIGURATION/OPEN, the media
channel, and audible SBC test-tone output. One apparent connection-time crash
was not reproducible on the subsequent run and has no crash artifact, so it is
recorded as an unresolved transient rather than attributed to the SBC change.
The staged `6c31e946…f24a0d8a` candidate retained Delay Report and applied the
bounded START burst. Device validation confirmed that it synchronized both TWS
ears, but measured about ten seconds from Starting to Streaming plus another
three seconds to audible output, and a second tone still could not start. The
current target-selected candidate `f50853d4…fd239779` therefore keeps the first
accepted AVDTP stream active between tone chunks, installs a new packetizer for
each Play, and reconnects only a peer-closed media L2CAP channel. It also moves
descriptor registration behind the identity-guarded Rust private ABI facade and
contains no firmware addresses in the C constructor. Host tests, full Canopus
CI, ARM build, build-plan, verifier, receipt signing, and watchface smoke tests
pass; repeat playback still requires device validation. Repeated clean
disconnect, second-session, and 95% reliability gates remain open.
