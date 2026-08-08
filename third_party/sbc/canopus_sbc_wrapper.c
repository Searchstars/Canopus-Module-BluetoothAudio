/*
 * Thin fixed-profile wrapper around BlueZ SBC (LGPL-2.1-or-later).
 * Caller owns all storage; no libc allocation is used.
 */
#include <stddef.h>
#include <stdint.h>

#include "sbc.h"

#define CANOPUS_SBC_ALIGN 16u
#define CANOPUS_SBC_PCM_FRAMES 128u
#define CANOPUS_SBC_CHANNELS 2u

struct canopus_sbc_encoder {
    sbc_t sbc;
};

uint32_t canopus_sbc_encoder_size(void)
{
    return (uint32_t)(CANOPUS_SBC_ALIGN - 1u +
                      sizeof(struct canopus_sbc_encoder) +
                      sbc_get_private_size());
}

void *canopus_sbc_encoder_init(void *memory, uint32_t memory_size,
                               uint8_t bitpool)
{
    uintptr_t base;
    struct canopus_sbc_encoder *encoder;
    uint8_t *private_storage;
    size_t private_size = sbc_get_private_size();

    if (memory == NULL || memory_size < canopus_sbc_encoder_size() ||
        bitpool < 2u || bitpool > 250u)
        return NULL;
    base = ((uintptr_t)memory + CANOPUS_SBC_ALIGN - 1u) &
           ~((uintptr_t)CANOPUS_SBC_ALIGN - 1u);
    encoder = (struct canopus_sbc_encoder *)base;
    private_storage = (uint8_t *)(encoder + 1);
    if (sbc_init_static(&encoder->sbc, private_storage, private_size, 0) != 0)
        return NULL;

    encoder->sbc.frequency = SBC_FREQ_44100;
    encoder->sbc.blocks = SBC_BLK_16;
    encoder->sbc.subbands = SBC_SB_8;
    encoder->sbc.mode = SBC_MODE_STEREO;
    encoder->sbc.allocation = SBC_AM_LOUDNESS;
    encoder->sbc.bitpool = bitpool;
    encoder->sbc.endian = SBC_LE;
    return encoder;
}

int32_t canopus_sbc_encoder_encode(void *context, const int16_t *pcm,
                                   uint32_t pcm_frames, uint8_t *output,
                                   uint32_t output_capacity)
{
    struct canopus_sbc_encoder *encoder = context;
    ssize_t written = 0;
    ssize_t consumed;

    if (encoder == NULL || pcm == NULL || output == NULL ||
        pcm_frames != CANOPUS_SBC_PCM_FRAMES)
        return -22;
    consumed = sbc_encode(&encoder->sbc, pcm,
                          CANOPUS_SBC_PCM_FRAMES * CANOPUS_SBC_CHANNELS *
                              sizeof(int16_t),
                          output, output_capacity, &written);
    if (consumed < 0)
        return (int32_t)consumed;
    if (consumed != (ssize_t)(CANOPUS_SBC_PCM_FRAMES * CANOPUS_SBC_CHANNELS *
                              sizeof(int16_t)) ||
        written <= 0 || written > (ssize_t)output_capacity)
        return -5;
    return (int32_t)written;
}
