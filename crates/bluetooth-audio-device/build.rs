fn main() {
    println!("cargo:rerun-if-changed=../../third_party/sbc");

    cc::Build::new()
        .file("../../third_party/sbc/sbc.c")
        .file("../../third_party/sbc/sbc_primitives.c")
        .file("../../third_party/sbc/canopus_sbc_wrapper.c")
        .include("../../third_party/sbc")
        .define("CANOPUS_FREESTANDING", "1")
        .flag("-ffreestanding")
        .flag("-fno-common")
        .flag("-fno-builtin")
        .flag("-Wno-unused-parameter")
        .flag("-Wno-ignored-attributes")
        .opt_level(3)
        .compile("canopus_sbc");
}
