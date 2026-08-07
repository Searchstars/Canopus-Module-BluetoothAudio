#include <stdint.h>

#include "canopus_module_registration.h"

#define CANOPUS_NUTTX_OPEN ((int (*)(const char *, int, ...))(uintptr_t)0x0C1C15B1u)
#define CANOPUS_NUTTX_CLOSE ((int (*)(int))(uintptr_t)0x0C1AAB71u)
#define CANOPUS_NUTTX_WRITE ((int32_t (*)(int, const void *, uint32_t))(uintptr_t)0x0C1C31C9u)
#define CANOPUS_O_WRONLY 2

static const char canopus_device_path[] = "/dev/canopus";

static void canopus_register_descriptor(void)
{
    extern const uint8_t canopus_module_descriptor[];
    extern const void *canopus_module_descriptor_ptr(void);
    static const struct canopus_module_registration_v1 registration = {
        CANOPUS_MODULE_REGISTRATION_MAGIC,
        (uint32_t)(uintptr_t)&canopus_module_descriptor,
        "bluetooth_audio",
    };
    int fd = CANOPUS_NUTTX_OPEN(canopus_device_path, CANOPUS_O_WRONLY);

    (void)canopus_module_descriptor_ptr();
    if (fd >= 0) {
        (void)CANOPUS_NUTTX_WRITE(fd, &registration, sizeof(registration));
        (void)CANOPUS_NUTTX_CLOSE(fd);
    }
}

/* Stock modlib loader glue; all module behavior is Rust. */
__attribute__((constructor)) static void canopus_mod_ctor(void)
{
    extern int canopus_mod_prepare(const void *);
    (void)canopus_mod_prepare(0);
    canopus_register_descriptor();
}

__attribute__((destructor)) static void canopus_mod_dtor(void)
{
    extern int canopus_mod_stop(const void *);
    (void)canopus_mod_stop(0);
}
