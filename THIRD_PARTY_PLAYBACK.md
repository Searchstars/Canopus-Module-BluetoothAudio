# Third-party Bluetooth audio playback interface

## 1. Scope

`BluetoothAudio` exposes a single-producer compressed-audio endpoint to Vela/NuttX applications:

```text
userspace
┌──────────────────────────────────────────────┐
│ Music app / third-party app                  │
│ open("/dev/canopus_audio", O_RDWR)           │
│ ioctl(CANOPUS_AUDIO_IOC_SET_FORMAT, MP3)     │
│ write(fd, mp3_bytes, length)                 │
│ ioctl(START / PAUSE / RESUME / STOP / DRAIN)│
└──────────────────────┬───────────────────────┘
                       │ character-device ABI
═══════════════════════╪════════════════════════
                       │ module / kernel context
                       ▼
              single-writer input ring
                       │
                  nanomp3 decoder
                       │ f32 decode output
                       ▼
              saturating S16 PCM conversion
                       │ 44.1 kHz mono/stereo
                       ▼
              fixed-profile SBC encoder
                       │ negotiated bitpool
                       ▼
                RTP/A2DP packetizer
                       │
                      AVDTP
                       │
                firmware L2CAP stack
                       │
                       ▼
                   headphones
```

The interface is intentionally an audio-data API, not a Bluetooth control API. Third-party applications never receive an adapter pointer, CID, callback address, target-private symbol, or headset address. Pairing and connection remain owned by the Headphones app.

The first implementation accepts MPEG Layer III and produces the already negotiated A2DP SBC profile: 44.1 kHz, stereo, 16 blocks, 8 subbands, Loudness allocation. Decoded 44.1-kHz streams pass through directly; decoded 24-kHz and 48-kHz streams use the same bounded, phase-carrying linear resampler exercised by the packaged long-audio and host fixtures. Other sample rates are rejected until additional bounded resampling ratios are added. Mono MP3 is duplicated to stereo.

## 2. Device and ownership

- Path: `/dev/canopus_audio`
- Access: read/write control and compressed-audio endpoint.
- Writers: exactly one open owner at a time. A second open returns `-EBUSY`.
- Readers: the owning descriptor may nonblockingly read fixed-size `canopus_audio_control_event_v1` headset media-control records.
- Lifetime: the device exists only while the boot-resident BluetoothAudio module is active.
- Headset prerequisite: a headset must already be connected and AVDTP must be OPEN or STREAMING before `START` can progress beyond `BUFFERING`.
- Closing the owning file descriptor performs `STOP`, discards queued input, resets the decoder, and releases writer ownership. It does not unpair or disconnect the headset.

One descriptor owns one playback session. Control ioctls issued through any non-owning descriptor fail.

## 3. ABI constants

The canonical C declarations live in `include/canopus_audio.h`. Every structure begins with `struct_size` and uses fixed-width integer fields; pointers never appear in stored status or ring data.

### Formats

| Value | Name | Meaning |
|---:|---|---|
| 1 | `CANOPUS_AUDIO_FORMAT_MP3` | MPEG Layer III byte stream decoded by `nanomp3` |
| 2 | `CANOPUS_AUDIO_FORMAT_PCM_S16LE` | Reserved for direct interleaved S16LE PCM |

ABI v1 implements MP3. Unsupported formats return `-EINVAL`.

### Ioctls

| Command | Argument | Meaning |
|---|---|---|
| `GET_ABI` | `uint32_t *` | Returns `CANOPUS_AUDIO_ABI_VERSION` |
| `SET_FORMAT` | `struct canopus_audio_format_v1 *` | Selects format before data is accepted |
| `START` | none | Starts when enough input and a headset are available |
| `PAUSE` | none | Stops decoder consumption and RTP production without discarding input |
| `RESUME` | none | Resumes a paused session; equivalent to START only from PAUSED |
| `STOP` | none | Stops playback and discards compressed/PCM/SBC buffered data |
| `DRAIN` | none | Declares end of input and plays all complete buffered frames |
| `FLUSH` | none | Discards queued input and resets decoder while retaining configuration |
| `GET_STATUS` | `struct canopus_audio_status_v1 *` | Returns a coherent status snapshot |
| `SET_VOLUME` | `uint32_t *` | Sets software PCM gain from 0 (mute) through 100 (unity) |
| `GET_VOLUME` | `uint32_t *` | Returns the current 0..100 volume percentage |

Volume is applied after MP3 decode and mono-to-stereo expansion, immediately
before SBC encoding. It is module-local software gain: it does not send AVRCP,
change the earbud's persistent hardware volume, or affect other Bluetooth
clients. Values above 100 are rejected to prevent amplification clipping. The
default for each new exclusive open is 100.

`SET_FORMAT` is legal only in `IDLE`, `CONFIGURED`, or `STOPPED`. Reconfiguration while playing returns `-EBUSY`.

### Headset media controls

Standard AVRCP AV/C PASS THROUGH press commands are acknowledged immediately and exposed to the owning application through `read()`:

```c
struct canopus_audio_control_event_v1 {
    uint32_t struct_size;
    uint32_t kind;       /* CANOPUS_AUDIO_CONTROL_* */
    uint32_t sequence;   /* wrapping, monotonically increasing */
    uint32_t reserved;   /* zero */
};
```

Supported kinds are `PLAY`, `PAUSE`, `NEXT`, and `PREVIOUS`. Release commands are acknowledged but do not produce a second event. Unknown operation IDs are answered `NOT_IMPLEMENTED` without affecting A2DP or absolute-volume transactions.

The queue contains eight records, preserves accepted press order, and drops the newest event when full. `read()` returns one complete record, `-EAGAIN` when empty, and `-EINVAL` for a null or undersized buffer. Opening and closing the exclusive session discard stale events. Applications should poll from their existing service loop; the Bluetooth callback never blocks waiting for a reader.

## 4. Format descriptor

```c
struct canopus_audio_format_v1 {
    uint32_t struct_size;
    uint32_t format;           /* CANOPUS_AUDIO_FORMAT_* */
    uint32_t sample_rate_hint; /* 0 = derive from stream */
    uint32_t channels_hint;    /* 0 = derive; otherwise 1 or 2 */
    uint32_t flags;            /* must be zero in ABI v1 */
    uint32_t reserved[3];      /* must be zero */
};
```

Hints do not override the MP3 frame header. If nonzero, they are assertions: a decoded stream that disagrees transitions to `ERROR` with `UNSUPPORTED_RATE` or `UNSUPPORTED_CHANNELS`.

## 5. Playback states

```text
CLOSED
  │ open
  ▼
IDLE ──SET_FORMAT──> CONFIGURED
                         │ START
                         ▼
                     BUFFERING ──────────────┐
                         │ enough MP3 data   │ underrun
                         ▼                   │
                      PLAYING ───────────────┘
                       │   │
                 PAUSE │   │ DRAIN + empty
                       ▼   ▼
                     PAUSED  DRAINING ──> STOPPED
                       │                    │
                    RESUME                 START
                       └──────────> BUFFERING

Any active state ──STOP──> STOPPED
Any configured state ──FLUSH──> CONFIGURED
Fatal decode/codec/transport failure ──> ERROR
```

`START` is asynchronous. Success from `ioctl` means the request was accepted, not that audio is already audible. Applications use `GET_STATUS` to observe `BUFFERING` and `PLAYING`.

Pause keeps compressed bytes already accepted by `write`. Stop and close discard them. Drain rejects subsequent writes until the session is stopped or reconfigured.

## 6. Write and backpressure semantics

`write(fd, data, count)` copies MP3 bytes into a bounded single-producer/single-consumer ring and returns immediately:

- returns `1..count` when that many bytes were accepted;
- may return a short count at ring wrap or under backpressure;
- returns `-EAGAIN` when no space is currently available;
- returns `-EPIPE` after `DRAIN` or when no format/session is configured;
- returns `-EIO` while the pipeline is in `ERROR`;
- a zero-length write returns zero.

Applications must retry short writes. ABI v1 does not block a userspace thread inside the Bluetooth callback domain and provides no unbounded allocation.

Recommended producer loop:

```c
while (remaining != 0) {
    ssize_t n = write(fd, cursor, remaining);
    if (n > 0) {
        cursor += n;
        remaining -= (size_t)n;
    } else if (n == -EAGAIN) {
        /* sleep/yield briefly, then retry */
    } else {
        /* query status and abort the session */
        break;
    }
}
```

The endpoint owns a 16 KiB compressed-input ring allocated when the driver is registered. The larger nanomp3, PCM, and SBC workspaces are allocated lazily on the first accepted `START` and then reused for later generations. `write` does not allocate module memory.

## 7. Decode and transport scheduling

The file-operation callbacks only validate, copy into the ring, change atomics, and enqueue owner work. They never:

- take the Bluetooth core lock while waiting;
- decode MP3 in the caller's context;
- send L2CAP packets directly;
- call LVGL;
- block waiting for ring space.

A Bluetooth-owner pump performs bounded work:

1. linearize enough ring bytes for `nanomp3::Decoder::decode`;
2. retain incomplete trailing MP3 data for the next write;
3. validate sample rate and channel count;
4. validate a decoded 24-kHz, 44.1-kHz, or 48-kHz stream and convert f32 samples to interleaved S16;
5. resample 24-kHz or 48-kHz PCM to 44.1 kHz with bounded phase-carrying linear interpolation;
6. duplicate mono to stereo;
7. feed exactly 128 PCM samples per channel into the SBC encoder;
8. aggregate negotiated SBC frames into RTP packets up to the media MTU;
9. pace packets on the existing Bluetooth timer and respect PAUSE/STOP generations.

The pump has a per-callback decode/frame budget so one application cannot monopolize the Bluetooth owner thread. If more work remains, it requeues itself.

## 8. Status and diagnostics

```c
struct canopus_audio_status_v1 {
    uint32_t struct_size;
    uint32_t abi_version;
    uint32_t state;
    int32_t  last_error;
    uint32_t format;
    uint32_t input_capacity;
    uint32_t input_used;
    uint32_t input_free;
    uint32_t decoded_sample_rate;
    uint32_t decoded_channels;
    uint32_t negotiated_bitpool;
    uint32_t bytes_accepted;
    uint32_t bytes_consumed;
    uint32_t pcm_frames;
    uint32_t rtp_packets;
    uint32_t underruns;
    uint32_t generation;
    uint32_t volume_percent;
};
```

Counters are wrapping 32-bit diagnostics. `generation` changes on START, STOP, FLUSH, close/open, and a new format; stale queued decoder/timer work compares its captured generation before touching the pipeline.

## 9. Error recovery

- Malformed MP3 prefixes and metadata are skipped only when the decoder reports a positive consumed length; a decoded rate other than 24 kHz, 44.1 kHz, or 48 kHz, an unsupported channel layout, or a codec failure enters `ERROR`. DRAIN discards only an incomplete trailing frame after all complete frames are sent.
- Input underrun: transitions PLAYING to BUFFERING and increments `underruns`; it does not tear down AVDTP.
- Media- or signaling-channel loss: active third-party playback enters `ERROR`; bounded input is discarded only by the caller's subsequent STOP/FLUSH/close.
- STOP/close: invalidates decoder and timer generations before clearing buffers.

## 10. Security and compatibility rules

1. The public header contains no firmware address and is target-independent.
2. Character-driver registration comes from the selected private ABI backend and occurs only after the exact firmware identity guard succeeds.
3. The module remains boot-resident after publishing file operations; callback pointers must never outlive module text.
4. Every ioctl validates `struct_size`, enum ranges, zero-reserved fields, and legal state transitions.
5. Ring indices are monotonic atomics and capacity is a power of two; the producer publishes bytes with Release and the consumer observes them with Acquire.
6. No command accepts a Bluetooth address, raw CID, function pointer, or arbitrary kernel pointer beyond the immediate ioctl argument structure.
7. ABI additions append fields or allocate new command values; existing values and field offsets never change.

## 11. Minimal third-party example

```c
#include "canopus_audio.h"
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>

int play_mp3(const void *data, unsigned size)
{
    const unsigned char *p = data;
    struct canopus_audio_format_v1 format = {
        .struct_size = sizeof(format),
        .format = CANOPUS_AUDIO_FORMAT_MP3,
    };
    int fd = open(CANOPUS_AUDIO_DEVICE_PATH, O_WRONLY);
    if (fd < 0) return fd;
    if (ioctl(fd, CANOPUS_AUDIO_IOC_SET_FORMAT,
              (unsigned long)&format) < 0) goto fail;
    if (ioctl(fd, CANOPUS_AUDIO_IOC_START, 0) < 0) goto fail;

    while (size != 0) {
        int n = write(fd, p, size);
        if (n > 0) {
            p += n;
            size -= (unsigned)n;
            continue;
        }
        if (n != -EAGAIN) goto fail;
        /* platform-specific short sleep/yield */
    }
    if (ioctl(fd, CANOPUS_AUDIO_IOC_DRAIN, 0) < 0) goto fail;
    close(fd);
    return 0;

fail:
    (void)ioctl(fd, CANOPUS_AUDIO_IOC_STOP, 0);
    close(fd);
    return -1;
}
```

Real applications should poll `GET_STATUS` until `STOPPED` before closing after DRAIN; closing immediately intentionally cancels and discards the tail.

## 12. Version-one device gates

Before this API is declared device-stable, validation must demonstrate:

1. exclusive open and correct release after close/crash;
2. short writes, wraparound, `-EAGAIN`, and no ring corruption;
3. arbitrarily chunked MP3 input, including frame headers split across writes;
4. 24-kHz, 44.1-kHz, and 48-kHz mono and stereo decoding, including bounded 24/48→44.1-kHz resampling;
5. decoder error recovery and unsupported-rate rejection;
6. START, repeated PAUSE/RESUME, STOP, FLUSH, and DRAIN;
7. repeated tracks without reconnecting or stale timer work;
8. input underrun recovery without left/right desynchronization;
9. clean behavior when the media or signaling channel disconnects;
10. verifier-clean ARM artifact within the exact target's loader bound;
11. a third-party sample app built only against `include/canopus_audio.h`;
12. long playback with bounded memory and no callback/core-lock drops.
