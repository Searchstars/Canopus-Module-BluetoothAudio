/* Stock modlib loader glue; all module behavior is Rust. */
__attribute__((constructor)) static void canopus_mod_ctor(void)
{
    extern int canopus_mod_prepare(const void *);
    extern const void *canopus_module_descriptor_ptr(void);
    (void)canopus_module_descriptor_ptr();
    (void)canopus_mod_prepare(0);
}

__attribute__((destructor)) static void canopus_mod_dtor(void)
{
    extern int canopus_mod_stop(const void *);
    (void)canopus_mod_stop(0);
}
