#ifndef CANOPUS_AUDIO_H
#define CANOPUS_AUDIO_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define CANOPUS_AUDIO_DEVICE_PATH "/dev/canopus_audio"
#define CANOPUS_AUDIO_ABI_VERSION UINT32_C(1)

#define CANOPUS_AUDIO_FORMAT_MP3 UINT32_C(1)
#define CANOPUS_AUDIO_FORMAT_PCM_S16LE UINT32_C(2) /* reserved in ABI v1 */

#define CANOPUS_AUDIO_STATE_CLOSED UINT32_C(0)
#define CANOPUS_AUDIO_STATE_IDLE UINT32_C(1)
#define CANOPUS_AUDIO_STATE_CONFIGURED UINT32_C(2)
#define CANOPUS_AUDIO_STATE_BUFFERING UINT32_C(3)
#define CANOPUS_AUDIO_STATE_PLAYING UINT32_C(4)
#define CANOPUS_AUDIO_STATE_PAUSED UINT32_C(5)
#define CANOPUS_AUDIO_STATE_DRAINING UINT32_C(6)
#define CANOPUS_AUDIO_STATE_STOPPED UINT32_C(7)
#define CANOPUS_AUDIO_STATE_ERROR UINT32_C(8)

/* Stable private command namespace: ASCII "CA" in the upper halfword. */
#define CANOPUS_AUDIO_IOC_GET_ABI UINT32_C(0x43410001)
#define CANOPUS_AUDIO_IOC_SET_FORMAT UINT32_C(0x43410002)
#define CANOPUS_AUDIO_IOC_START UINT32_C(0x43410003)
#define CANOPUS_AUDIO_IOC_PAUSE UINT32_C(0x43410004)
#define CANOPUS_AUDIO_IOC_RESUME UINT32_C(0x43410005)
#define CANOPUS_AUDIO_IOC_STOP UINT32_C(0x43410006)
#define CANOPUS_AUDIO_IOC_DRAIN UINT32_C(0x43410007)
#define CANOPUS_AUDIO_IOC_FLUSH UINT32_C(0x43410008)
#define CANOPUS_AUDIO_IOC_GET_STATUS UINT32_C(0x43410009)
#define CANOPUS_AUDIO_IOC_SET_VOLUME UINT32_C(0x4341000a)
#define CANOPUS_AUDIO_IOC_GET_VOLUME UINT32_C(0x4341000b)

#define CANOPUS_AUDIO_VOLUME_MIN UINT32_C(0)
#define CANOPUS_AUDIO_VOLUME_MAX UINT32_C(100)
#define CANOPUS_AUDIO_VOLUME_DEFAULT UINT32_C(100)

struct canopus_audio_format_v1 {
    uint32_t struct_size;
    uint32_t format;
    uint32_t sample_rate_hint;
    uint32_t channels_hint;
    uint32_t flags;
    uint32_t reserved[3];
};

struct canopus_audio_status_v1 {
    uint32_t struct_size;
    uint32_t abi_version;
    uint32_t state;
    int32_t last_error;
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

#ifdef __cplusplus
}
#endif

#endif /* CANOPUS_AUDIO_H */
