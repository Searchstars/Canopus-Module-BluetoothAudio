/* Stock modlib loader glue; target-private calls remain in Rust. */
#include <stdint.h>

/* Anchors for build-generated fixups that decode opaque codec-table words
 * before Rust or the decoder can observe .rodata. The module is copied into
 * writable RAM by stock modlib even though these sections are read-only ELF
 * inputs. */
__attribute__((section(".rodata"), used, aligned(4)))
const uint8_t canopus_rodata_anchor[4] = {0};
__attribute__((section(".rodata.cst16"), used, aligned(16)))
const uint8_t canopus_rodata_cst16_anchor[16] = {0};
__attribute__((section(".rodata.analysis_consts_fixed4_simd_odd"), used,
               aligned(4)))
const uint8_t canopus_analysis_consts_fixed4_simd_odd_anchor[4] = {0};
__attribute__((section(".rodata.analysis_consts_fixed4_simd_even"), used,
               aligned(4)))
const uint8_t canopus_analysis_consts_fixed4_simd_even_anchor[4] = {0};
__attribute__((section(".rodata.analysis_consts_fixed8_simd_odd"), used,
               aligned(4)))
const uint8_t canopus_analysis_consts_fixed8_simd_odd_anchor[4] = {0};
__attribute__((section(".rodata.analysis_consts_fixed8_simd_even"), used,
               aligned(4)))
const uint8_t canopus_analysis_consts_fixed8_simd_even_anchor[4] = {0};

extern void canopus_decode_opaque_words(void) __attribute__((weak));

__attribute__((constructor)) static void canopus_mod_ctor(void)
{
    extern int canopus_mod_prepare(const void *);
    extern int canopus_register_module_descriptor(void);

    if (canopus_decode_opaque_words != 0) {
        canopus_decode_opaque_words();
    }
    (void)canopus_mod_prepare(0);
    (void)canopus_register_module_descriptor();
}

__attribute__((destructor)) static void canopus_mod_dtor(void)
{
    extern int canopus_mod_stop(const void *);
    (void)canopus_mod_stop(0);
}
