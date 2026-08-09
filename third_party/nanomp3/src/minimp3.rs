#![allow(
    clippy::all,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unused_assignments,
    unused_mut
)]

#[inline(always)]
unsafe fn memcpy(dst: *mut (), src: *const (), count: usize) {
    core::ptr::copy_nonoverlapping(src as *const u8, dst as *mut u8, count);
}

type int16_t = i16;
type int32_t = i32;
type uint8_t = u8;
type uint16_t = u16;
type uint32_t = u32;
#[derive(Copy, Clone, Default)]
#[repr(C)]
pub struct mp3dec_frame_info_t {
    pub frame_bytes: i32,
    pub frame_offset: i32,
    pub channels: i32,
    pub hz: i32,
    pub layer: i32,
    pub bitrate_kbps: i32,
}
#[derive(Copy, Clone)]
#[repr(C)]
pub struct mp3dec_t {
    mdct_overlap: [[f32; 288]; 2],
    qmf_state: [f32; 960],
    reserv: i32,
    free_format_bytes: i32,
    header: [u8; 4],
    reserv_buf: [u8; 511],
}

impl mp3dec_t {
    pub const fn new() -> Self {
        Self {
            mdct_overlap: [[0.; 288]; 2],
            qmf_state: [0.; 960],
            reserv: 0,
            free_format_bytes: 0,
            header: [0; 4],
            reserv_buf: [0; 511],
        }
    }
}

type mp3d_sample_t = f32;
#[derive(Copy, Clone)]
#[repr(C)]
pub struct mp3dec_scratch_t {
    bs: bs_t,
    maindata: [uint8_t; 2815],
    gr_info: [L3_gr_info_t; 4],
    grbuf: [[f32; 576]; 2],
    scf: [f32; 40],
    syn: [[f32; 64]; 33],
    ist_pos: [[uint8_t; 39]; 2],
}
#[derive(Copy, Clone)]
#[repr(C)]
struct L3_gr_info_t {
    sfbtab: *const uint8_t,
    part_23_length: uint16_t,
    big_values: uint16_t,
    scalefac_compress: uint16_t,
    global_gain: uint8_t,
    block_type: uint8_t,
    mixed_block_flag: uint8_t,
    n_long_sfb: uint8_t,
    n_short_sfb: uint8_t,
    table_select: [uint8_t; 3],
    region_count: [uint8_t; 3],
    subblock_gain: [uint8_t; 3],
    preflag: uint8_t,
    scalefac_scale: uint8_t,
    count1_table: uint8_t,
    scfsi: uint8_t,
}
#[derive(Copy, Clone)]
#[repr(C)]
struct bs_t {
    buf: *const uint8_t,
    pos: i32,
    limit: i32,
}

impl mp3dec_scratch_t {
    pub const fn new() -> Self {
        Self {
            bs: bs_t {
                buf: core::ptr::null(),
                pos: 0,
                limit: 0,
            },
            maindata: [0; 2815],
            gr_info: [L3_gr_info_t {
                sfbtab: core::ptr::null(),
                part_23_length: 0,
                big_values: 0,
                scalefac_compress: 0,
                global_gain: 0,
                block_type: 0,
                mixed_block_flag: 0,
                n_long_sfb: 0,
                n_short_sfb: 0,
                table_select: [0; 3],
                region_count: [0; 3],
                subblock_gain: [0; 3],
                preflag: 0,
                scalefac_scale: 0,
                count1_table: 0,
                scfsi: 0,
            }; 4],
            grbuf: [[0.0; 576]; 2],
            scf: [0.0; 40],
            syn: [[0.0; 64]; 33],
            ist_pos: [[0; 39]; 2],
        }
    }
}
unsafe fn bs_init(mut bs: *mut bs_t, mut data: *const uint8_t, mut bytes: i32) {
    (*bs).buf = data;
    (*bs).pos = 0 as i32;
    (*bs).limit = bytes * 8 as i32;
}
unsafe fn get_bits(mut bs: *mut bs_t, mut n: i32) -> uint32_t {
    let mut next: uint32_t = 0;
    let mut cache: uint32_t = 0 as i32 as uint32_t;
    let mut s: uint32_t = ((*bs).pos & 7 as i32) as uint32_t;
    let mut shl: i32 = (n as uint32_t).wrapping_add(s) as i32;
    let mut p: *const uint8_t = ((*bs).buf).offset(((*bs).pos >> 3 as i32) as isize);
    (*bs).pos += n;
    if (*bs).pos > (*bs).limit {
        return 0 as i32 as uint32_t;
    }
    let fresh0 = p;
    p = p.offset(1);
    next = (*fresh0 as i32 & 255 as i32 >> s) as uint32_t;
    loop {
        shl -= 8 as i32;
        if !(shl > 0 as i32) {
            break;
        }
        cache |= next << shl;
        let fresh1 = p;
        p = p.offset(1);
        next = *fresh1 as uint32_t;
    }
    return cache | next >> -shl;
}
unsafe fn hdr_valid(mut h: *const uint8_t) -> i32 {
    return (*h.offset(0 as i32 as isize) as i32 == 0xff as i32
        && (*h.offset(1 as i32 as isize) as i32 & 0xf0 as i32 == 0xf0 as i32
            || *h.offset(1 as i32 as isize) as i32 & 0xfe as i32 == 0xe2 as i32)
        && *h.offset(1 as i32 as isize) as i32 >> 1 as i32 & 3 as i32 != 0 as i32
        && *h.offset(2 as i32 as isize) as i32 >> 4 as i32 != 15 as i32
        && *h.offset(2 as i32 as isize) as i32 >> 2 as i32 & 3 as i32 != 3 as i32)
        as i32;
}
unsafe fn hdr_compare(mut h1: *const uint8_t, mut h2: *const uint8_t) -> i32 {
    return (hdr_valid(h2) != 0
        && (*h1.offset(1 as i32 as isize) as i32 ^ *h2.offset(1 as i32 as isize) as i32)
            & 0xfe as i32
            == 0 as i32
        && (*h1.offset(2 as i32 as isize) as i32 ^ *h2.offset(2 as i32 as isize) as i32)
            & 0xc as i32
            == 0 as i32
        && (*h1.offset(2 as i32 as isize) as i32 & 0xf0 as i32 == 0 as i32) as i32
            ^ (*h2.offset(2 as i32 as isize) as i32 & 0xf0 as i32 == 0 as i32) as i32
            == 0) as i32;
}
unsafe fn hdr_bitrate_kbps(mut h: *const uint8_t) -> u32 {
    static halfrate: [[[uint8_t; 15]; 3]; 2] = [
        [
            [
                0 as i32 as uint8_t,
                4 as i32 as uint8_t,
                8 as i32 as uint8_t,
                12 as i32 as uint8_t,
                16 as i32 as uint8_t,
                20 as i32 as uint8_t,
                24 as i32 as uint8_t,
                28 as i32 as uint8_t,
                32 as i32 as uint8_t,
                40 as i32 as uint8_t,
                48 as i32 as uint8_t,
                56 as i32 as uint8_t,
                64 as i32 as uint8_t,
                72 as i32 as uint8_t,
                80 as i32 as uint8_t,
            ],
            [
                0 as i32 as uint8_t,
                4 as i32 as uint8_t,
                8 as i32 as uint8_t,
                12 as i32 as uint8_t,
                16 as i32 as uint8_t,
                20 as i32 as uint8_t,
                24 as i32 as uint8_t,
                28 as i32 as uint8_t,
                32 as i32 as uint8_t,
                40 as i32 as uint8_t,
                48 as i32 as uint8_t,
                56 as i32 as uint8_t,
                64 as i32 as uint8_t,
                72 as i32 as uint8_t,
                80 as i32 as uint8_t,
            ],
            [
                0 as i32 as uint8_t,
                16 as i32 as uint8_t,
                24 as i32 as uint8_t,
                28 as i32 as uint8_t,
                32 as i32 as uint8_t,
                40 as i32 as uint8_t,
                48 as i32 as uint8_t,
                56 as i32 as uint8_t,
                64 as i32 as uint8_t,
                72 as i32 as uint8_t,
                80 as i32 as uint8_t,
                88 as i32 as uint8_t,
                96 as i32 as uint8_t,
                112 as i32 as uint8_t,
                128 as i32 as uint8_t,
            ],
        ],
        [
            [
                0 as i32 as uint8_t,
                16 as i32 as uint8_t,
                20 as i32 as uint8_t,
                24 as i32 as uint8_t,
                28 as i32 as uint8_t,
                32 as i32 as uint8_t,
                40 as i32 as uint8_t,
                48 as i32 as uint8_t,
                56 as i32 as uint8_t,
                64 as i32 as uint8_t,
                80 as i32 as uint8_t,
                96 as i32 as uint8_t,
                112 as i32 as uint8_t,
                128 as i32 as uint8_t,
                160 as i32 as uint8_t,
            ],
            [
                0 as i32 as uint8_t,
                16 as i32 as uint8_t,
                24 as i32 as uint8_t,
                28 as i32 as uint8_t,
                32 as i32 as uint8_t,
                40 as i32 as uint8_t,
                48 as i32 as uint8_t,
                56 as i32 as uint8_t,
                64 as i32 as uint8_t,
                80 as i32 as uint8_t,
                96 as i32 as uint8_t,
                112 as i32 as uint8_t,
                128 as i32 as uint8_t,
                160 as i32 as uint8_t,
                192 as i32 as uint8_t,
            ],
            [
                0 as i32 as uint8_t,
                16 as i32 as uint8_t,
                32 as i32 as uint8_t,
                48 as i32 as uint8_t,
                64 as i32 as uint8_t,
                80 as i32 as uint8_t,
                96 as i32 as uint8_t,
                112 as i32 as uint8_t,
                128 as i32 as uint8_t,
                144 as i32 as uint8_t,
                160 as i32 as uint8_t,
                176 as i32 as uint8_t,
                192 as i32 as uint8_t,
                208 as i32 as uint8_t,
                224 as i32 as uint8_t,
            ],
        ],
    ];
    return (2 as i32
        * halfrate[(*h.offset(1 as i32 as isize) as i32 & 0x8 as i32 != 0) as i32 as usize]
            [((*h.offset(1 as i32 as isize) as i32 >> 1 as i32 & 3 as i32) - 1 as i32) as usize]
            [(*h.offset(2 as i32 as isize) as i32 >> 4 as i32) as usize] as i32) as u32;
}
unsafe fn hdr_sample_rate_hz(mut h: *const uint8_t) -> u32 {
    static g_hz: [u32; 3] = [
        44100 as i32 as u32,
        48000 as i32 as u32,
        32000 as i32 as u32,
    ];
    return g_hz[(*h.offset(2 as i32 as isize) as i32 >> 2 as i32 & 3 as i32) as usize]
        >> (*h.offset(1 as i32 as isize) as i32 & 0x8 as i32 == 0) as i32
        >> (*h.offset(1 as i32 as isize) as i32 & 0x10 as i32 == 0) as i32;
}
unsafe fn hdr_frame_samples(mut h: *const uint8_t) -> u32 {
    return (if *h.offset(1 as i32 as isize) as i32 & 6 as i32 == 6 as i32 {
        384 as i32
    } else {
        1152 as i32 >> (*h.offset(1 as i32 as isize) as i32 & 14 as i32 == 2 as i32) as i32
    }) as u32;
}
unsafe fn hdr_frame_bytes(mut h: *const uint8_t, mut free_format_size: i32) -> i32 {
    let mut frame_bytes: i32 = (hdr_frame_samples(h))
        .wrapping_mul(hdr_bitrate_kbps(h))
        .wrapping_mul(125 as i32 as u32)
        .wrapping_div(hdr_sample_rate_hz(h)) as i32;
    if *h.offset(1 as i32 as isize) as i32 & 6 as i32 == 6 as i32 {
        frame_bytes &= !(3 as i32);
    }
    return if frame_bytes != 0 {
        frame_bytes
    } else {
        free_format_size
    };
}
unsafe fn hdr_padding(mut h: *const uint8_t) -> i32 {
    return if *h.offset(2 as i32 as isize) as i32 & 0x2 as i32 != 0 {
        if *h.offset(1 as i32 as isize) as i32 & 6 as i32 == 6 as i32 {
            4 as i32
        } else {
            1 as i32
        }
    } else {
        0 as i32
    };
}
unsafe fn L3_read_side_info(
    mut bs: *mut bs_t,
    mut gr: *mut L3_gr_info_t,
    mut hdr: *const uint8_t,
) -> i32 {
    static g_scf_long: [[uint8_t; 23]; 8] = [
        [
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            14 as i32 as uint8_t,
            16 as i32 as uint8_t,
            20 as i32 as uint8_t,
            24 as i32 as uint8_t,
            28 as i32 as uint8_t,
            32 as i32 as uint8_t,
            38 as i32 as uint8_t,
            46 as i32 as uint8_t,
            52 as i32 as uint8_t,
            60 as i32 as uint8_t,
            68 as i32 as uint8_t,
            58 as i32 as uint8_t,
            54 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            16 as i32 as uint8_t,
            20 as i32 as uint8_t,
            24 as i32 as uint8_t,
            28 as i32 as uint8_t,
            32 as i32 as uint8_t,
            40 as i32 as uint8_t,
            48 as i32 as uint8_t,
            56 as i32 as uint8_t,
            64 as i32 as uint8_t,
            76 as i32 as uint8_t,
            90 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            14 as i32 as uint8_t,
            16 as i32 as uint8_t,
            20 as i32 as uint8_t,
            24 as i32 as uint8_t,
            28 as i32 as uint8_t,
            32 as i32 as uint8_t,
            38 as i32 as uint8_t,
            46 as i32 as uint8_t,
            52 as i32 as uint8_t,
            60 as i32 as uint8_t,
            68 as i32 as uint8_t,
            58 as i32 as uint8_t,
            54 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            14 as i32 as uint8_t,
            16 as i32 as uint8_t,
            18 as i32 as uint8_t,
            22 as i32 as uint8_t,
            26 as i32 as uint8_t,
            32 as i32 as uint8_t,
            38 as i32 as uint8_t,
            46 as i32 as uint8_t,
            54 as i32 as uint8_t,
            62 as i32 as uint8_t,
            70 as i32 as uint8_t,
            76 as i32 as uint8_t,
            36 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            14 as i32 as uint8_t,
            16 as i32 as uint8_t,
            20 as i32 as uint8_t,
            24 as i32 as uint8_t,
            28 as i32 as uint8_t,
            32 as i32 as uint8_t,
            38 as i32 as uint8_t,
            46 as i32 as uint8_t,
            52 as i32 as uint8_t,
            60 as i32 as uint8_t,
            68 as i32 as uint8_t,
            58 as i32 as uint8_t,
            54 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            16 as i32 as uint8_t,
            20 as i32 as uint8_t,
            24 as i32 as uint8_t,
            28 as i32 as uint8_t,
            34 as i32 as uint8_t,
            42 as i32 as uint8_t,
            50 as i32 as uint8_t,
            54 as i32 as uint8_t,
            76 as i32 as uint8_t,
            158 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            16 as i32 as uint8_t,
            18 as i32 as uint8_t,
            22 as i32 as uint8_t,
            28 as i32 as uint8_t,
            34 as i32 as uint8_t,
            40 as i32 as uint8_t,
            46 as i32 as uint8_t,
            54 as i32 as uint8_t,
            54 as i32 as uint8_t,
            192 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            16 as i32 as uint8_t,
            20 as i32 as uint8_t,
            24 as i32 as uint8_t,
            30 as i32 as uint8_t,
            38 as i32 as uint8_t,
            46 as i32 as uint8_t,
            56 as i32 as uint8_t,
            68 as i32 as uint8_t,
            84 as i32 as uint8_t,
            102 as i32 as uint8_t,
            26 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
    ];
    static g_scf_short: [[uint8_t; 40]; 8] = [
        [
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            24 as i32 as uint8_t,
            24 as i32 as uint8_t,
            24 as i32 as uint8_t,
            30 as i32 as uint8_t,
            30 as i32 as uint8_t,
            30 as i32 as uint8_t,
            40 as i32 as uint8_t,
            40 as i32 as uint8_t,
            40 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            16 as i32 as uint8_t,
            16 as i32 as uint8_t,
            16 as i32 as uint8_t,
            20 as i32 as uint8_t,
            20 as i32 as uint8_t,
            20 as i32 as uint8_t,
            24 as i32 as uint8_t,
            24 as i32 as uint8_t,
            24 as i32 as uint8_t,
            28 as i32 as uint8_t,
            28 as i32 as uint8_t,
            28 as i32 as uint8_t,
            36 as i32 as uint8_t,
            36 as i32 as uint8_t,
            36 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            26 as i32 as uint8_t,
            26 as i32 as uint8_t,
            26 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            26 as i32 as uint8_t,
            26 as i32 as uint8_t,
            26 as i32 as uint8_t,
            32 as i32 as uint8_t,
            32 as i32 as uint8_t,
            32 as i32 as uint8_t,
            42 as i32 as uint8_t,
            42 as i32 as uint8_t,
            42 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            24 as i32 as uint8_t,
            24 as i32 as uint8_t,
            24 as i32 as uint8_t,
            32 as i32 as uint8_t,
            32 as i32 as uint8_t,
            32 as i32 as uint8_t,
            44 as i32 as uint8_t,
            44 as i32 as uint8_t,
            44 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            24 as i32 as uint8_t,
            24 as i32 as uint8_t,
            24 as i32 as uint8_t,
            30 as i32 as uint8_t,
            30 as i32 as uint8_t,
            30 as i32 as uint8_t,
            40 as i32 as uint8_t,
            40 as i32 as uint8_t,
            40 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            22 as i32 as uint8_t,
            22 as i32 as uint8_t,
            22 as i32 as uint8_t,
            30 as i32 as uint8_t,
            30 as i32 as uint8_t,
            30 as i32 as uint8_t,
            56 as i32 as uint8_t,
            56 as i32 as uint8_t,
            56 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            16 as i32 as uint8_t,
            16 as i32 as uint8_t,
            16 as i32 as uint8_t,
            20 as i32 as uint8_t,
            20 as i32 as uint8_t,
            20 as i32 as uint8_t,
            26 as i32 as uint8_t,
            26 as i32 as uint8_t,
            26 as i32 as uint8_t,
            66 as i32 as uint8_t,
            66 as i32 as uint8_t,
            66 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            16 as i32 as uint8_t,
            16 as i32 as uint8_t,
            16 as i32 as uint8_t,
            20 as i32 as uint8_t,
            20 as i32 as uint8_t,
            20 as i32 as uint8_t,
            26 as i32 as uint8_t,
            26 as i32 as uint8_t,
            26 as i32 as uint8_t,
            34 as i32 as uint8_t,
            34 as i32 as uint8_t,
            34 as i32 as uint8_t,
            42 as i32 as uint8_t,
            42 as i32 as uint8_t,
            42 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
    ];
    static g_scf_mixed: [[uint8_t; 40]; 8] = [
        [
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            24 as i32 as uint8_t,
            24 as i32 as uint8_t,
            24 as i32 as uint8_t,
            30 as i32 as uint8_t,
            30 as i32 as uint8_t,
            30 as i32 as uint8_t,
            40 as i32 as uint8_t,
            40 as i32 as uint8_t,
            40 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            0 as i32 as uint8_t,
            0,
            0,
            0,
        ],
        [
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            16 as i32 as uint8_t,
            16 as i32 as uint8_t,
            16 as i32 as uint8_t,
            20 as i32 as uint8_t,
            20 as i32 as uint8_t,
            20 as i32 as uint8_t,
            24 as i32 as uint8_t,
            24 as i32 as uint8_t,
            24 as i32 as uint8_t,
            28 as i32 as uint8_t,
            28 as i32 as uint8_t,
            28 as i32 as uint8_t,
            36 as i32 as uint8_t,
            36 as i32 as uint8_t,
            36 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            26 as i32 as uint8_t,
            26 as i32 as uint8_t,
            26 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            26 as i32 as uint8_t,
            26 as i32 as uint8_t,
            26 as i32 as uint8_t,
            32 as i32 as uint8_t,
            32 as i32 as uint8_t,
            32 as i32 as uint8_t,
            42 as i32 as uint8_t,
            42 as i32 as uint8_t,
            42 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            0 as i32 as uint8_t,
            0,
            0,
            0,
        ],
        [
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            24 as i32 as uint8_t,
            24 as i32 as uint8_t,
            24 as i32 as uint8_t,
            32 as i32 as uint8_t,
            32 as i32 as uint8_t,
            32 as i32 as uint8_t,
            44 as i32 as uint8_t,
            44 as i32 as uint8_t,
            44 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            0 as i32 as uint8_t,
            0,
            0,
            0,
        ],
        [
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            24 as i32 as uint8_t,
            24 as i32 as uint8_t,
            24 as i32 as uint8_t,
            30 as i32 as uint8_t,
            30 as i32 as uint8_t,
            30 as i32 as uint8_t,
            40 as i32 as uint8_t,
            40 as i32 as uint8_t,
            40 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            0 as i32 as uint8_t,
            0,
            0,
            0,
        ],
        [
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            22 as i32 as uint8_t,
            22 as i32 as uint8_t,
            22 as i32 as uint8_t,
            30 as i32 as uint8_t,
            30 as i32 as uint8_t,
            30 as i32 as uint8_t,
            56 as i32 as uint8_t,
            56 as i32 as uint8_t,
            56 as i32 as uint8_t,
            0 as i32 as uint8_t,
            0,
        ],
        [
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            10 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            14 as i32 as uint8_t,
            16 as i32 as uint8_t,
            16 as i32 as uint8_t,
            16 as i32 as uint8_t,
            20 as i32 as uint8_t,
            20 as i32 as uint8_t,
            20 as i32 as uint8_t,
            26 as i32 as uint8_t,
            26 as i32 as uint8_t,
            26 as i32 as uint8_t,
            66 as i32 as uint8_t,
            66 as i32 as uint8_t,
            66 as i32 as uint8_t,
            0 as i32 as uint8_t,
            0,
        ],
        [
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            16 as i32 as uint8_t,
            16 as i32 as uint8_t,
            16 as i32 as uint8_t,
            20 as i32 as uint8_t,
            20 as i32 as uint8_t,
            20 as i32 as uint8_t,
            26 as i32 as uint8_t,
            26 as i32 as uint8_t,
            26 as i32 as uint8_t,
            34 as i32 as uint8_t,
            34 as i32 as uint8_t,
            34 as i32 as uint8_t,
            42 as i32 as uint8_t,
            42 as i32 as uint8_t,
            42 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            0 as i32 as uint8_t,
            0,
        ],
    ];
    let mut tables: u32 = 0;
    let mut scfsi: u32 = 0 as i32 as u32;
    let mut main_data_begin: i32 = 0;
    let mut part_23_sum: i32 = 0 as i32;
    let mut sr_idx: i32 = (*hdr.offset(2 as i32 as isize) as i32 >> 2 as i32 & 3 as i32)
        + ((*hdr.offset(1 as i32 as isize) as i32 >> 3 as i32 & 1 as i32)
            + (*hdr.offset(1 as i32 as isize) as i32 >> 4 as i32 & 1 as i32))
            * 3 as i32;
    sr_idx -= (sr_idx != 0 as i32) as i32;
    let mut gr_count: i32 = if *hdr.offset(3 as i32 as isize) as i32 & 0xc0 as i32 == 0xc0 as i32 {
        1 as i32
    } else {
        2 as i32
    };
    if *hdr.offset(1 as i32 as isize) as i32 & 0x8 as i32 != 0 {
        gr_count *= 2 as i32;
        main_data_begin = get_bits(bs, 9 as i32) as i32;
        scfsi = get_bits(bs, 7 as i32 + gr_count);
    } else {
        main_data_begin = (get_bits(bs, 8 as i32 + gr_count) >> gr_count) as i32;
    }
    loop {
        if *hdr.offset(3 as i32 as isize) as i32 & 0xc0 as i32 == 0xc0 as i32 {
            scfsi <<= 4 as i32;
        }
        (*gr).part_23_length = get_bits(bs, 12 as i32) as uint16_t;
        part_23_sum += (*gr).part_23_length as i32;
        (*gr).big_values = get_bits(bs, 9 as i32) as uint16_t;
        if (*gr).big_values as i32 > 288 as i32 {
            return -(1 as i32);
        }
        (*gr).global_gain = get_bits(bs, 8 as i32) as uint8_t;
        (*gr).scalefac_compress = get_bits(
            bs,
            if *hdr.offset(1 as i32 as isize) as i32 & 0x8 as i32 != 0 {
                4 as i32
            } else {
                9 as i32
            },
        ) as uint16_t;
        (*gr).sfbtab = (g_scf_long[sr_idx as usize]).as_ptr();
        (*gr).n_long_sfb = 22 as i32 as uint8_t;
        (*gr).n_short_sfb = 0 as i32 as uint8_t;
        if get_bits(bs, 1 as i32) != 0 {
            (*gr).block_type = get_bits(bs, 2 as i32) as uint8_t;
            if (*gr).block_type == 0 {
                return -(1 as i32);
            }
            (*gr).mixed_block_flag = get_bits(bs, 1 as i32) as uint8_t;
            (*gr).region_count[0 as i32 as usize] = 7 as i32 as uint8_t;
            (*gr).region_count[1 as i32 as usize] = 255 as i32 as uint8_t;
            if (*gr).block_type as i32 == 2 as i32 {
                scfsi &= 0xf0f as i32 as u32;
                if (*gr).mixed_block_flag == 0 {
                    (*gr).region_count[0 as i32 as usize] = 8 as i32 as uint8_t;
                    (*gr).sfbtab = (g_scf_short[sr_idx as usize]).as_ptr();
                    (*gr).n_long_sfb = 0 as i32 as uint8_t;
                    (*gr).n_short_sfb = 39 as i32 as uint8_t;
                } else {
                    (*gr).sfbtab = (g_scf_mixed[sr_idx as usize]).as_ptr();
                    (*gr).n_long_sfb = (if *hdr.offset(1 as i32 as isize) as i32 & 0x8 as i32 != 0 {
                        8 as i32
                    } else {
                        6 as i32
                    }) as uint8_t;
                    (*gr).n_short_sfb = 30 as i32 as uint8_t;
                }
            }
            tables = get_bits(bs, 10 as i32);
            tables <<= 5 as i32;
            (*gr).subblock_gain[0 as i32 as usize] = get_bits(bs, 3 as i32) as uint8_t;
            (*gr).subblock_gain[1 as i32 as usize] = get_bits(bs, 3 as i32) as uint8_t;
            (*gr).subblock_gain[2 as i32 as usize] = get_bits(bs, 3 as i32) as uint8_t;
        } else {
            (*gr).block_type = 0 as i32 as uint8_t;
            (*gr).mixed_block_flag = 0 as i32 as uint8_t;
            tables = get_bits(bs, 15 as i32);
            (*gr).region_count[0 as i32 as usize] = get_bits(bs, 4 as i32) as uint8_t;
            (*gr).region_count[1 as i32 as usize] = get_bits(bs, 3 as i32) as uint8_t;
            (*gr).region_count[2 as i32 as usize] = 255 as i32 as uint8_t;
        }
        (*gr).table_select[0 as i32 as usize] = (tables >> 10 as i32) as uint8_t;
        (*gr).table_select[1 as i32 as usize] = (tables >> 5 as i32 & 31 as i32 as u32) as uint8_t;
        (*gr).table_select[2 as i32 as usize] = (tables & 31 as i32 as u32) as uint8_t;
        (*gr).preflag = (if *hdr.offset(1 as i32 as isize) as i32 & 0x8 as i32 != 0 {
            get_bits(bs, 1 as i32)
        } else {
            ((*gr).scalefac_compress as i32 >= 500 as i32) as i32 as uint32_t
        }) as uint8_t;
        (*gr).scalefac_scale = get_bits(bs, 1 as i32) as uint8_t;
        (*gr).count1_table = get_bits(bs, 1 as i32) as uint8_t;
        (*gr).scfsi = (scfsi >> 12 as i32 & 15 as i32 as u32) as uint8_t;
        scfsi <<= 4 as i32;
        gr = gr.offset(1);
        gr_count -= 1;
        if !(gr_count != 0) {
            break;
        }
    }
    if part_23_sum + (*bs).pos > (*bs).limit + main_data_begin * 8 as i32 {
        return -(1 as i32);
    }
    return main_data_begin;
}
unsafe fn L3_read_scalefactors(
    mut scf: *mut uint8_t,
    mut ist_pos: *mut uint8_t,
    mut scf_size: *const uint8_t,
    mut scf_count: *const uint8_t,
    mut bitbuf: *mut bs_t,
    mut scfsi: i32,
) {
    let mut i: i32 = 0;
    let mut k: i32 = 0;
    i = 0 as i32;
    while i < 4 as i32 && *scf_count.offset(i as isize) as i32 != 0 {
        let mut cnt: i32 = *scf_count.offset(i as isize) as i32;
        if scfsi & 8 as i32 != 0 {
            memcpy(scf as *mut (), ist_pos as *const (), cnt as usize);
        } else {
            let mut bits: i32 = *scf_size.offset(i as isize) as i32;
            if bits == 0 {
                core::ptr::write_bytes(scf, 0, cnt as usize);
                core::ptr::write_bytes(ist_pos, 0, cnt as usize);
            } else {
                let mut max_scf: i32 = if scfsi < 0 as i32 {
                    ((1 as i32) << bits) - 1 as i32
                } else {
                    -(1 as i32)
                };
                k = 0 as i32;
                while k < cnt {
                    let mut s: i32 = get_bits(bitbuf, bits) as i32;
                    *ist_pos.offset(k as isize) =
                        (if s == max_scf { -(1 as i32) } else { s }) as uint8_t;
                    *scf.offset(k as isize) = s as uint8_t;
                    k += 1;
                }
            }
        }
        ist_pos = ist_pos.offset(cnt as isize);
        scf = scf.offset(cnt as isize);
        i += 1;
        scfsi *= 2 as i32;
    }
    let ref mut fresh2 = *scf.offset(2 as i32 as isize);
    *fresh2 = 0 as i32 as uint8_t;
    let ref mut fresh3 = *scf.offset(1 as i32 as isize);
    *fresh3 = *fresh2;
    *scf.offset(0 as i32 as isize) = *fresh3;
}
unsafe fn L3_ldexp_q2(mut y: f32, mut exp_q2: i32) -> f32 {
    static g_expfrac: [f32; 4] = [
        9.31322575e-10f32,
        7.83145814e-10f32,
        6.58544508e-10f32,
        5.53767716e-10f32,
    ];
    let mut e: i32 = 0;
    loop {
        e = if 30 as i32 * 4 as i32 > exp_q2 {
            exp_q2
        } else {
            30 as i32 * 4 as i32
        };
        y *= g_expfrac[(e & 3 as i32) as usize]
            * ((1 as i32) << 30 as i32 >> (e >> 2 as i32)) as f32;
        exp_q2 -= e;
        if !(exp_q2 > 0 as i32) {
            break;
        }
    }
    return y;
}
unsafe fn L3_decode_scalefactors(
    mut hdr: *const uint8_t,
    mut ist_pos: *mut uint8_t,
    mut bs: *mut bs_t,
    mut gr: *const L3_gr_info_t,
    mut scf: *mut f32,
    mut ch: i32,
) {
    static g_scf_partitions: [[uint8_t; 28]; 3] = [
        [
            6 as i32 as uint8_t,
            5 as i32 as uint8_t,
            5 as i32 as uint8_t,
            5 as i32 as uint8_t,
            6 as i32 as uint8_t,
            5 as i32 as uint8_t,
            5 as i32 as uint8_t,
            5 as i32 as uint8_t,
            6 as i32 as uint8_t,
            5 as i32 as uint8_t,
            7 as i32 as uint8_t,
            3 as i32 as uint8_t,
            11 as i32 as uint8_t,
            10 as i32 as uint8_t,
            0 as i32 as uint8_t,
            0 as i32 as uint8_t,
            7 as i32 as uint8_t,
            7 as i32 as uint8_t,
            7 as i32 as uint8_t,
            0 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            3 as i32 as uint8_t,
            8 as i32 as uint8_t,
            8 as i32 as uint8_t,
            5 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            8 as i32 as uint8_t,
            9 as i32 as uint8_t,
            6 as i32 as uint8_t,
            12 as i32 as uint8_t,
            6 as i32 as uint8_t,
            9 as i32 as uint8_t,
            9 as i32 as uint8_t,
            9 as i32 as uint8_t,
            6 as i32 as uint8_t,
            9 as i32 as uint8_t,
            12 as i32 as uint8_t,
            6 as i32 as uint8_t,
            15 as i32 as uint8_t,
            18 as i32 as uint8_t,
            0 as i32 as uint8_t,
            0 as i32 as uint8_t,
            6 as i32 as uint8_t,
            15 as i32 as uint8_t,
            12 as i32 as uint8_t,
            0 as i32 as uint8_t,
            6 as i32 as uint8_t,
            12 as i32 as uint8_t,
            9 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            18 as i32 as uint8_t,
            9 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
        [
            9 as i32 as uint8_t,
            9 as i32 as uint8_t,
            6 as i32 as uint8_t,
            12 as i32 as uint8_t,
            9 as i32 as uint8_t,
            9 as i32 as uint8_t,
            9 as i32 as uint8_t,
            9 as i32 as uint8_t,
            9 as i32 as uint8_t,
            9 as i32 as uint8_t,
            12 as i32 as uint8_t,
            6 as i32 as uint8_t,
            18 as i32 as uint8_t,
            18 as i32 as uint8_t,
            0 as i32 as uint8_t,
            0 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            12 as i32 as uint8_t,
            0 as i32 as uint8_t,
            12 as i32 as uint8_t,
            9 as i32 as uint8_t,
            9 as i32 as uint8_t,
            6 as i32 as uint8_t,
            15 as i32 as uint8_t,
            12 as i32 as uint8_t,
            9 as i32 as uint8_t,
            0 as i32 as uint8_t,
        ],
    ];
    let mut scf_partition: *const uint8_t = (g_scf_partitions
        [(((*gr).n_short_sfb != 0) as i32 + ((*gr).n_long_sfb == 0) as i32) as usize])
        .as_ptr();
    let mut scf_size: [uint8_t; 4] = [0; 4];
    let mut iscf: [uint8_t; 40] = [0; 40];
    let mut i: i32 = 0;
    let mut scf_shift: i32 = (*gr).scalefac_scale as i32 + 1 as i32;
    let mut gain_exp: i32 = 0;
    let mut scfsi: i32 = (*gr).scfsi as i32;
    let mut gain: f32 = 0.;
    if *hdr.offset(1 as i32 as isize) as i32 & 0x8 as i32 != 0 {
        static g_scfc_decode: [uint8_t; 16] = [
            0 as i32 as uint8_t,
            1 as i32 as uint8_t,
            2 as i32 as uint8_t,
            3 as i32 as uint8_t,
            12 as i32 as uint8_t,
            5 as i32 as uint8_t,
            6 as i32 as uint8_t,
            7 as i32 as uint8_t,
            9 as i32 as uint8_t,
            10 as i32 as uint8_t,
            11 as i32 as uint8_t,
            13 as i32 as uint8_t,
            14 as i32 as uint8_t,
            15 as i32 as uint8_t,
            18 as i32 as uint8_t,
            19 as i32 as uint8_t,
        ];
        let mut part: i32 = g_scfc_decode[(*gr).scalefac_compress as usize] as i32;
        scf_size[0 as i32 as usize] = (part >> 2 as i32) as uint8_t;
        scf_size[1 as i32 as usize] = scf_size[0 as i32 as usize];
        scf_size[2 as i32 as usize] = (part & 3 as i32) as uint8_t;
        scf_size[3 as i32 as usize] = scf_size[2 as i32 as usize];
    } else {
        static g_mod: [uint8_t; 24] = [
            5 as i32 as uint8_t,
            5 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            5 as i32 as uint8_t,
            5 as i32 as uint8_t,
            4 as i32 as uint8_t,
            1 as i32 as uint8_t,
            4 as i32 as uint8_t,
            3 as i32 as uint8_t,
            1 as i32 as uint8_t,
            1 as i32 as uint8_t,
            5 as i32 as uint8_t,
            6 as i32 as uint8_t,
            6 as i32 as uint8_t,
            1 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            4 as i32 as uint8_t,
            1 as i32 as uint8_t,
            4 as i32 as uint8_t,
            3 as i32 as uint8_t,
            1 as i32 as uint8_t,
            1 as i32 as uint8_t,
        ];
        let mut k: i32 = 0;
        let mut modprod: i32 = 0;
        let mut sfc: i32 = 0;
        let mut ist: i32 =
            (*hdr.offset(3 as i32 as isize) as i32 & 0x10 as i32 != 0 && ch != 0) as i32;
        sfc = (*gr).scalefac_compress as i32 >> ist;
        k = ist * 3 as i32 * 4 as i32;
        while sfc >= 0 as i32 {
            modprod = 1 as i32;
            i = 3 as i32;
            while i >= 0 as i32 {
                scf_size[i as usize] = (sfc / modprod % g_mod[(k + i) as usize] as i32) as uint8_t;
                modprod *= g_mod[(k + i) as usize] as i32;
                i -= 1;
            }
            sfc -= modprod;
            k += 4 as i32;
        }
        scf_partition = scf_partition.offset(k as isize);
        scfsi = -(16 as i32);
    }
    L3_read_scalefactors(
        iscf.as_mut_ptr(),
        ist_pos,
        scf_size.as_mut_ptr(),
        scf_partition,
        bs,
        scfsi,
    );
    if (*gr).n_short_sfb != 0 {
        let mut sh: i32 = 3 as i32 - scf_shift;
        i = 0 as i32;
        while i < (*gr).n_short_sfb as i32 {
            iscf[((*gr).n_long_sfb as i32 + i + 0 as i32) as usize] =
                (iscf[((*gr).n_long_sfb as i32 + i + 0 as i32) as usize] as i32
                    + (((*gr).subblock_gain[0 as i32 as usize] as i32) << sh))
                    as uint8_t;
            iscf[((*gr).n_long_sfb as i32 + i + 1 as i32) as usize] =
                (iscf[((*gr).n_long_sfb as i32 + i + 1 as i32) as usize] as i32
                    + (((*gr).subblock_gain[1 as i32 as usize] as i32) << sh))
                    as uint8_t;
            iscf[((*gr).n_long_sfb as i32 + i + 2 as i32) as usize] =
                (iscf[((*gr).n_long_sfb as i32 + i + 2 as i32) as usize] as i32
                    + (((*gr).subblock_gain[2 as i32 as usize] as i32) << sh))
                    as uint8_t;
            i += 3 as i32;
        }
    } else if (*gr).preflag != 0 {
        static g_preamp: [uint8_t; 10] = [
            1 as i32 as uint8_t,
            1 as i32 as uint8_t,
            1 as i32 as uint8_t,
            1 as i32 as uint8_t,
            2 as i32 as uint8_t,
            2 as i32 as uint8_t,
            3 as i32 as uint8_t,
            3 as i32 as uint8_t,
            3 as i32 as uint8_t,
            2 as i32 as uint8_t,
        ];
        i = 0 as i32;
        while i < 10 as i32 {
            iscf[(11 as i32 + i) as usize] =
                (iscf[(11 as i32 + i) as usize] as i32 + g_preamp[i as usize] as i32) as uint8_t;
            i += 1;
        }
    }
    gain_exp = (*gr).global_gain as i32 + -(1 as i32) * 4 as i32
        - 210 as i32
        - (if *hdr.offset(3 as i32 as isize) as i32 & 0xe0 as i32 == 0x60 as i32 {
            2 as i32
        } else {
            0 as i32
        });
    gain = L3_ldexp_q2(
        ((1 as i32)
            << (255 as i32 + -(1 as i32) * 4 as i32 - 210 as i32 + 3 as i32 & !(3 as i32))
                / 4 as i32) as f32,
        (255 as i32 + -(1 as i32) * 4 as i32 - 210 as i32 + 3 as i32 & !(3 as i32)) - gain_exp,
    );
    i = 0 as i32;
    while i < (*gr).n_long_sfb as i32 + (*gr).n_short_sfb as i32 {
        *scf.offset(i as isize) = L3_ldexp_q2(gain, (iscf[i as usize] as i32) << scf_shift);
        i += 1;
    }
}
static g_pow43: [f32; 145] = [
    0 as i32 as f32,
    -(1 as i32) as f32,
    -2.519842f32,
    -4.326749f32,
    -6.349604f32,
    -8.549880f32,
    -10.902724f32,
    -13.390518f32,
    -16.000000f32,
    -18.720754f32,
    -21.544347f32,
    -24.463781f32,
    -27.473142f32,
    -30.567351f32,
    -33.741992f32,
    -36.993181f32,
    0 as i32 as f32,
    1 as i32 as f32,
    2.519842f32,
    4.326749f32,
    6.349604f32,
    8.549880f32,
    10.902724f32,
    13.390518f32,
    16.000000f32,
    18.720754f32,
    21.544347f32,
    24.463781f32,
    27.473142f32,
    30.567351f32,
    33.741992f32,
    36.993181f32,
    40.317474f32,
    43.711787f32,
    47.173345f32,
    50.699631f32,
    54.288352f32,
    57.937408f32,
    61.644865f32,
    65.408941f32,
    69.227979f32,
    73.100443f32,
    77.024898f32,
    81.000000f32,
    85.024491f32,
    89.097188f32,
    93.216975f32,
    97.382800f32,
    101.593667f32,
    105.848633f32,
    110.146801f32,
    114.487321f32,
    118.869381f32,
    123.292209f32,
    127.755065f32,
    132.257246f32,
    136.798076f32,
    141.376907f32,
    145.993119f32,
    150.646117f32,
    155.335327f32,
    160.060199f32,
    164.820202f32,
    169.614826f32,
    174.443577f32,
    179.305980f32,
    184.201575f32,
    189.129918f32,
    194.090580f32,
    199.083145f32,
    204.107210f32,
    209.162385f32,
    214.248292f32,
    219.364564f32,
    224.510845f32,
    229.686789f32,
    234.892058f32,
    240.126328f32,
    245.389280f32,
    250.680604f32,
    256.000000f32,
    261.347174f32,
    266.721841f32,
    272.123723f32,
    277.552547f32,
    283.008049f32,
    288.489971f32,
    293.998060f32,
    299.532071f32,
    305.091761f32,
    310.676898f32,
    316.287249f32,
    321.922592f32,
    327.582707f32,
    333.267377f32,
    338.976394f32,
    344.709550f32,
    350.466646f32,
    356.247482f32,
    362.051866f32,
    367.879608f32,
    373.730522f32,
    379.604427f32,
    385.501143f32,
    391.420496f32,
    397.362314f32,
    403.326427f32,
    409.312672f32,
    415.320884f32,
    421.350905f32,
    427.402579f32,
    433.475750f32,
    439.570269f32,
    445.685987f32,
    451.822757f32,
    457.980436f32,
    464.158883f32,
    470.357960f32,
    476.577530f32,
    482.817459f32,
    489.077615f32,
    495.357868f32,
    501.658090f32,
    507.978156f32,
    514.317941f32,
    520.677324f32,
    527.056184f32,
    533.454404f32,
    539.871867f32,
    546.308458f32,
    552.764065f32,
    559.238575f32,
    565.731879f32,
    572.243870f32,
    578.774440f32,
    585.323483f32,
    591.890898f32,
    598.476581f32,
    605.080431f32,
    611.702349f32,
    618.342238f32,
    625.000000f32,
    631.675540f32,
    638.368763f32,
    645.079578f32,
];
unsafe fn L3_pow_43(mut x: i32) -> f32 {
    let mut frac: f32 = 0.;
    let mut sign: i32 = 0;
    let mut mult: i32 = 256 as i32;
    if x < 129 as i32 {
        return g_pow43[(16 as i32 + x) as usize];
    }
    if x < 1024 as i32 {
        mult = 16 as i32;
        x <<= 3 as i32;
    }
    sign = 2 as i32 * x & 64 as i32;
    frac = ((x & 63 as i32) - sign) as f32 / ((x & !(63 as i32)) + sign) as f32;
    return g_pow43[(16 as i32 + (x + sign >> 6 as i32)) as usize]
        * (1.0f32 + frac * (4.0f32 / 3 as i32 as f32 + frac * (2.0f32 / 9 as i32 as f32)))
        * mult as f32;
}
unsafe fn L3_huffman(
    mut dst: *mut f32,
    mut bs: *mut bs_t,
    mut gr_info: *const L3_gr_info_t,
    mut scf: *const f32,
    mut layer3gr_limit: i32,
) {
    static tabs: [int16_t; 2164] = [
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        0 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        513 as i32 as int16_t,
        513 as i32 as int16_t,
        513 as i32 as int16_t,
        513 as i32 as int16_t,
        513 as i32 as int16_t,
        513 as i32 as int16_t,
        513 as i32 as int16_t,
        513 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        -(255 as i32) as int16_t,
        1313 as i32 as int16_t,
        1298 as i32 as int16_t,
        1282 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        290 as i32 as int16_t,
        288 as i32 as int16_t,
        -(255 as i32) as int16_t,
        1313 as i32 as int16_t,
        1298 as i32 as int16_t,
        1282 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        528 as i32 as int16_t,
        528 as i32 as int16_t,
        528 as i32 as int16_t,
        528 as i32 as int16_t,
        528 as i32 as int16_t,
        528 as i32 as int16_t,
        528 as i32 as int16_t,
        528 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        290 as i32 as int16_t,
        288 as i32 as int16_t,
        -(253 as i32) as int16_t,
        -(318 as i32) as int16_t,
        -(351 as i32) as int16_t,
        -(367 as i32) as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        819 as i32 as int16_t,
        818 as i32 as int16_t,
        547 as i32 as int16_t,
        547 as i32 as int16_t,
        275 as i32 as int16_t,
        275 as i32 as int16_t,
        275 as i32 as int16_t,
        275 as i32 as int16_t,
        561 as i32 as int16_t,
        560 as i32 as int16_t,
        515 as i32 as int16_t,
        546 as i32 as int16_t,
        289 as i32 as int16_t,
        274 as i32 as int16_t,
        288 as i32 as int16_t,
        258 as i32 as int16_t,
        -(254 as i32) as int16_t,
        -(287 as i32) as int16_t,
        1329 as i32 as int16_t,
        1299 as i32 as int16_t,
        1314 as i32 as int16_t,
        1312 as i32 as int16_t,
        1057 as i32 as int16_t,
        1057 as i32 as int16_t,
        1042 as i32 as int16_t,
        1042 as i32 as int16_t,
        1026 as i32 as int16_t,
        1026 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        768 as i32 as int16_t,
        768 as i32 as int16_t,
        768 as i32 as int16_t,
        768 as i32 as int16_t,
        563 as i32 as int16_t,
        560 as i32 as int16_t,
        306 as i32 as int16_t,
        306 as i32 as int16_t,
        291 as i32 as int16_t,
        259 as i32 as int16_t,
        -(252 as i32) as int16_t,
        -(413 as i32) as int16_t,
        -(477 as i32) as int16_t,
        -(542 as i32) as int16_t,
        1298 as i32 as int16_t,
        -(575 as i32) as int16_t,
        1041 as i32 as int16_t,
        1041 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        -(383 as i32) as int16_t,
        -(399 as i32) as int16_t,
        1107 as i32 as int16_t,
        1092 as i32 as int16_t,
        1106 as i32 as int16_t,
        1061 as i32 as int16_t,
        849 as i32 as int16_t,
        849 as i32 as int16_t,
        789 as i32 as int16_t,
        789 as i32 as int16_t,
        1104 as i32 as int16_t,
        1091 as i32 as int16_t,
        773 as i32 as int16_t,
        773 as i32 as int16_t,
        1076 as i32 as int16_t,
        1075 as i32 as int16_t,
        341 as i32 as int16_t,
        340 as i32 as int16_t,
        325 as i32 as int16_t,
        309 as i32 as int16_t,
        834 as i32 as int16_t,
        804 as i32 as int16_t,
        577 as i32 as int16_t,
        577 as i32 as int16_t,
        532 as i32 as int16_t,
        532 as i32 as int16_t,
        516 as i32 as int16_t,
        516 as i32 as int16_t,
        832 as i32 as int16_t,
        818 as i32 as int16_t,
        803 as i32 as int16_t,
        816 as i32 as int16_t,
        561 as i32 as int16_t,
        561 as i32 as int16_t,
        531 as i32 as int16_t,
        531 as i32 as int16_t,
        515 as i32 as int16_t,
        546 as i32 as int16_t,
        289 as i32 as int16_t,
        289 as i32 as int16_t,
        288 as i32 as int16_t,
        258 as i32 as int16_t,
        -(252 as i32) as int16_t,
        -(429 as i32) as int16_t,
        -(493 as i32) as int16_t,
        -(559 as i32) as int16_t,
        1057 as i32 as int16_t,
        1057 as i32 as int16_t,
        1042 as i32 as int16_t,
        1042 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        529 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        -(382 as i32) as int16_t,
        1077 as i32 as int16_t,
        -(415 as i32) as int16_t,
        1106 as i32 as int16_t,
        1061 as i32 as int16_t,
        1104 as i32 as int16_t,
        849 as i32 as int16_t,
        849 as i32 as int16_t,
        789 as i32 as int16_t,
        789 as i32 as int16_t,
        1091 as i32 as int16_t,
        1076 as i32 as int16_t,
        1029 as i32 as int16_t,
        1075 as i32 as int16_t,
        834 as i32 as int16_t,
        834 as i32 as int16_t,
        597 as i32 as int16_t,
        581 as i32 as int16_t,
        340 as i32 as int16_t,
        340 as i32 as int16_t,
        339 as i32 as int16_t,
        324 as i32 as int16_t,
        804 as i32 as int16_t,
        833 as i32 as int16_t,
        532 as i32 as int16_t,
        532 as i32 as int16_t,
        832 as i32 as int16_t,
        772 as i32 as int16_t,
        818 as i32 as int16_t,
        803 as i32 as int16_t,
        817 as i32 as int16_t,
        787 as i32 as int16_t,
        816 as i32 as int16_t,
        771 as i32 as int16_t,
        290 as i32 as int16_t,
        290 as i32 as int16_t,
        290 as i32 as int16_t,
        290 as i32 as int16_t,
        288 as i32 as int16_t,
        258 as i32 as int16_t,
        -(253 as i32) as int16_t,
        -(349 as i32) as int16_t,
        -(414 as i32) as int16_t,
        -(447 as i32) as int16_t,
        -(463 as i32) as int16_t,
        1329 as i32 as int16_t,
        1299 as i32 as int16_t,
        -(479 as i32) as int16_t,
        1314 as i32 as int16_t,
        1312 as i32 as int16_t,
        1057 as i32 as int16_t,
        1057 as i32 as int16_t,
        1042 as i32 as int16_t,
        1042 as i32 as int16_t,
        1026 as i32 as int16_t,
        1026 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        768 as i32 as int16_t,
        768 as i32 as int16_t,
        768 as i32 as int16_t,
        768 as i32 as int16_t,
        -(319 as i32) as int16_t,
        851 as i32 as int16_t,
        821 as i32 as int16_t,
        -(335 as i32) as int16_t,
        836 as i32 as int16_t,
        850 as i32 as int16_t,
        805 as i32 as int16_t,
        849 as i32 as int16_t,
        341 as i32 as int16_t,
        340 as i32 as int16_t,
        325 as i32 as int16_t,
        336 as i32 as int16_t,
        533 as i32 as int16_t,
        533 as i32 as int16_t,
        579 as i32 as int16_t,
        579 as i32 as int16_t,
        564 as i32 as int16_t,
        564 as i32 as int16_t,
        773 as i32 as int16_t,
        832 as i32 as int16_t,
        578 as i32 as int16_t,
        548 as i32 as int16_t,
        563 as i32 as int16_t,
        516 as i32 as int16_t,
        321 as i32 as int16_t,
        276 as i32 as int16_t,
        306 as i32 as int16_t,
        291 as i32 as int16_t,
        304 as i32 as int16_t,
        259 as i32 as int16_t,
        -(251 as i32) as int16_t,
        -(572 as i32) as int16_t,
        -(733 as i32) as int16_t,
        -(830 as i32) as int16_t,
        -(863 as i32) as int16_t,
        -(879 as i32) as int16_t,
        1041 as i32 as int16_t,
        1041 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        -(511 as i32) as int16_t,
        -(527 as i32) as int16_t,
        -(543 as i32) as int16_t,
        1396 as i32 as int16_t,
        1351 as i32 as int16_t,
        1381 as i32 as int16_t,
        1366 as i32 as int16_t,
        1395 as i32 as int16_t,
        1335 as i32 as int16_t,
        1380 as i32 as int16_t,
        -(559 as i32) as int16_t,
        1334 as i32 as int16_t,
        1138 as i32 as int16_t,
        1138 as i32 as int16_t,
        1063 as i32 as int16_t,
        1063 as i32 as int16_t,
        1350 as i32 as int16_t,
        1392 as i32 as int16_t,
        1031 as i32 as int16_t,
        1031 as i32 as int16_t,
        1062 as i32 as int16_t,
        1062 as i32 as int16_t,
        1364 as i32 as int16_t,
        1363 as i32 as int16_t,
        1120 as i32 as int16_t,
        1120 as i32 as int16_t,
        1333 as i32 as int16_t,
        1348 as i32 as int16_t,
        881 as i32 as int16_t,
        881 as i32 as int16_t,
        881 as i32 as int16_t,
        881 as i32 as int16_t,
        375 as i32 as int16_t,
        374 as i32 as int16_t,
        359 as i32 as int16_t,
        373 as i32 as int16_t,
        343 as i32 as int16_t,
        358 as i32 as int16_t,
        341 as i32 as int16_t,
        325 as i32 as int16_t,
        791 as i32 as int16_t,
        791 as i32 as int16_t,
        1123 as i32 as int16_t,
        1122 as i32 as int16_t,
        -(703 as i32) as int16_t,
        1105 as i32 as int16_t,
        1045 as i32 as int16_t,
        -(719 as i32) as int16_t,
        865 as i32 as int16_t,
        865 as i32 as int16_t,
        790 as i32 as int16_t,
        790 as i32 as int16_t,
        774 as i32 as int16_t,
        774 as i32 as int16_t,
        1104 as i32 as int16_t,
        1029 as i32 as int16_t,
        338 as i32 as int16_t,
        293 as i32 as int16_t,
        323 as i32 as int16_t,
        308 as i32 as int16_t,
        -(799 as i32) as int16_t,
        -(815 as i32) as int16_t,
        833 as i32 as int16_t,
        788 as i32 as int16_t,
        772 as i32 as int16_t,
        818 as i32 as int16_t,
        803 as i32 as int16_t,
        816 as i32 as int16_t,
        322 as i32 as int16_t,
        292 as i32 as int16_t,
        307 as i32 as int16_t,
        320 as i32 as int16_t,
        561 as i32 as int16_t,
        531 as i32 as int16_t,
        515 as i32 as int16_t,
        546 as i32 as int16_t,
        289 as i32 as int16_t,
        274 as i32 as int16_t,
        288 as i32 as int16_t,
        258 as i32 as int16_t,
        -(251 as i32) as int16_t,
        -(525 as i32) as int16_t,
        -(605 as i32) as int16_t,
        -(685 as i32) as int16_t,
        -(765 as i32) as int16_t,
        -(831 as i32) as int16_t,
        -(846 as i32) as int16_t,
        1298 as i32 as int16_t,
        1057 as i32 as int16_t,
        1057 as i32 as int16_t,
        1312 as i32 as int16_t,
        1282 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        512 as i32 as int16_t,
        1399 as i32 as int16_t,
        1398 as i32 as int16_t,
        1383 as i32 as int16_t,
        1367 as i32 as int16_t,
        1382 as i32 as int16_t,
        1396 as i32 as int16_t,
        1351 as i32 as int16_t,
        -(511 as i32) as int16_t,
        1381 as i32 as int16_t,
        1366 as i32 as int16_t,
        1139 as i32 as int16_t,
        1139 as i32 as int16_t,
        1079 as i32 as int16_t,
        1079 as i32 as int16_t,
        1124 as i32 as int16_t,
        1124 as i32 as int16_t,
        1364 as i32 as int16_t,
        1349 as i32 as int16_t,
        1363 as i32 as int16_t,
        1333 as i32 as int16_t,
        882 as i32 as int16_t,
        882 as i32 as int16_t,
        882 as i32 as int16_t,
        882 as i32 as int16_t,
        807 as i32 as int16_t,
        807 as i32 as int16_t,
        807 as i32 as int16_t,
        807 as i32 as int16_t,
        1094 as i32 as int16_t,
        1094 as i32 as int16_t,
        1136 as i32 as int16_t,
        1136 as i32 as int16_t,
        373 as i32 as int16_t,
        341 as i32 as int16_t,
        535 as i32 as int16_t,
        535 as i32 as int16_t,
        881 as i32 as int16_t,
        775 as i32 as int16_t,
        867 as i32 as int16_t,
        822 as i32 as int16_t,
        774 as i32 as int16_t,
        -(591 as i32) as int16_t,
        324 as i32 as int16_t,
        338 as i32 as int16_t,
        -(671 as i32) as int16_t,
        849 as i32 as int16_t,
        550 as i32 as int16_t,
        550 as i32 as int16_t,
        866 as i32 as int16_t,
        864 as i32 as int16_t,
        609 as i32 as int16_t,
        609 as i32 as int16_t,
        293 as i32 as int16_t,
        336 as i32 as int16_t,
        534 as i32 as int16_t,
        534 as i32 as int16_t,
        789 as i32 as int16_t,
        835 as i32 as int16_t,
        773 as i32 as int16_t,
        -(751 as i32) as int16_t,
        834 as i32 as int16_t,
        804 as i32 as int16_t,
        308 as i32 as int16_t,
        307 as i32 as int16_t,
        833 as i32 as int16_t,
        788 as i32 as int16_t,
        832 as i32 as int16_t,
        772 as i32 as int16_t,
        562 as i32 as int16_t,
        562 as i32 as int16_t,
        547 as i32 as int16_t,
        547 as i32 as int16_t,
        305 as i32 as int16_t,
        275 as i32 as int16_t,
        560 as i32 as int16_t,
        515 as i32 as int16_t,
        290 as i32 as int16_t,
        290 as i32 as int16_t,
        -(252 as i32) as int16_t,
        -(397 as i32) as int16_t,
        -(477 as i32) as int16_t,
        -(557 as i32) as int16_t,
        -(622 as i32) as int16_t,
        -(653 as i32) as int16_t,
        -(719 as i32) as int16_t,
        -(735 as i32) as int16_t,
        -(750 as i32) as int16_t,
        1329 as i32 as int16_t,
        1299 as i32 as int16_t,
        1314 as i32 as int16_t,
        1057 as i32 as int16_t,
        1057 as i32 as int16_t,
        1042 as i32 as int16_t,
        1042 as i32 as int16_t,
        1312 as i32 as int16_t,
        1282 as i32 as int16_t,
        1024 as i32 as int16_t,
        1024 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        784 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        -(383 as i32) as int16_t,
        1127 as i32 as int16_t,
        1141 as i32 as int16_t,
        1111 as i32 as int16_t,
        1126 as i32 as int16_t,
        1140 as i32 as int16_t,
        1095 as i32 as int16_t,
        1110 as i32 as int16_t,
        869 as i32 as int16_t,
        869 as i32 as int16_t,
        883 as i32 as int16_t,
        883 as i32 as int16_t,
        1079 as i32 as int16_t,
        1109 as i32 as int16_t,
        882 as i32 as int16_t,
        882 as i32 as int16_t,
        375 as i32 as int16_t,
        374 as i32 as int16_t,
        807 as i32 as int16_t,
        868 as i32 as int16_t,
        838 as i32 as int16_t,
        881 as i32 as int16_t,
        791 as i32 as int16_t,
        -(463 as i32) as int16_t,
        867 as i32 as int16_t,
        822 as i32 as int16_t,
        368 as i32 as int16_t,
        263 as i32 as int16_t,
        852 as i32 as int16_t,
        837 as i32 as int16_t,
        836 as i32 as int16_t,
        -(543 as i32) as int16_t,
        610 as i32 as int16_t,
        610 as i32 as int16_t,
        550 as i32 as int16_t,
        550 as i32 as int16_t,
        352 as i32 as int16_t,
        336 as i32 as int16_t,
        534 as i32 as int16_t,
        534 as i32 as int16_t,
        865 as i32 as int16_t,
        774 as i32 as int16_t,
        851 as i32 as int16_t,
        821 as i32 as int16_t,
        850 as i32 as int16_t,
        805 as i32 as int16_t,
        593 as i32 as int16_t,
        533 as i32 as int16_t,
        579 as i32 as int16_t,
        564 as i32 as int16_t,
        773 as i32 as int16_t,
        832 as i32 as int16_t,
        578 as i32 as int16_t,
        578 as i32 as int16_t,
        548 as i32 as int16_t,
        548 as i32 as int16_t,
        577 as i32 as int16_t,
        577 as i32 as int16_t,
        307 as i32 as int16_t,
        276 as i32 as int16_t,
        306 as i32 as int16_t,
        291 as i32 as int16_t,
        516 as i32 as int16_t,
        560 as i32 as int16_t,
        259 as i32 as int16_t,
        259 as i32 as int16_t,
        -(250 as i32) as int16_t,
        -(2107 as i32) as int16_t,
        -(2507 as i32) as int16_t,
        -(2764 as i32) as int16_t,
        -(2909 as i32) as int16_t,
        -(2974 as i32) as int16_t,
        -(3007 as i32) as int16_t,
        -(3023 as i32) as int16_t,
        1041 as i32 as int16_t,
        1041 as i32 as int16_t,
        1040 as i32 as int16_t,
        1040 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        -(767 as i32) as int16_t,
        -(1052 as i32) as int16_t,
        -(1213 as i32) as int16_t,
        -(1277 as i32) as int16_t,
        -(1358 as i32) as int16_t,
        -(1405 as i32) as int16_t,
        -(1469 as i32) as int16_t,
        -(1535 as i32) as int16_t,
        -(1550 as i32) as int16_t,
        -(1582 as i32) as int16_t,
        -(1614 as i32) as int16_t,
        -(1647 as i32) as int16_t,
        -(1662 as i32) as int16_t,
        -(1694 as i32) as int16_t,
        -(1726 as i32) as int16_t,
        -(1759 as i32) as int16_t,
        -(1774 as i32) as int16_t,
        -(1807 as i32) as int16_t,
        -(1822 as i32) as int16_t,
        -(1854 as i32) as int16_t,
        -(1886 as i32) as int16_t,
        1565 as i32 as int16_t,
        -(1919 as i32) as int16_t,
        -(1935 as i32) as int16_t,
        -(1951 as i32) as int16_t,
        -(1967 as i32) as int16_t,
        1731 as i32 as int16_t,
        1730 as i32 as int16_t,
        1580 as i32 as int16_t,
        1717 as i32 as int16_t,
        -(1983 as i32) as int16_t,
        1729 as i32 as int16_t,
        1564 as i32 as int16_t,
        -(1999 as i32) as int16_t,
        1548 as i32 as int16_t,
        -(2015 as i32) as int16_t,
        -(2031 as i32) as int16_t,
        1715 as i32 as int16_t,
        1595 as i32 as int16_t,
        -(2047 as i32) as int16_t,
        1714 as i32 as int16_t,
        -(2063 as i32) as int16_t,
        1610 as i32 as int16_t,
        -(2079 as i32) as int16_t,
        1609 as i32 as int16_t,
        -(2095 as i32) as int16_t,
        1323 as i32 as int16_t,
        1323 as i32 as int16_t,
        1457 as i32 as int16_t,
        1457 as i32 as int16_t,
        1307 as i32 as int16_t,
        1307 as i32 as int16_t,
        1712 as i32 as int16_t,
        1547 as i32 as int16_t,
        1641 as i32 as int16_t,
        1700 as i32 as int16_t,
        1699 as i32 as int16_t,
        1594 as i32 as int16_t,
        1685 as i32 as int16_t,
        1625 as i32 as int16_t,
        1442 as i32 as int16_t,
        1442 as i32 as int16_t,
        1322 as i32 as int16_t,
        1322 as i32 as int16_t,
        -(780 as i32) as int16_t,
        -(973 as i32) as int16_t,
        -(910 as i32) as int16_t,
        1279 as i32 as int16_t,
        1278 as i32 as int16_t,
        1277 as i32 as int16_t,
        1262 as i32 as int16_t,
        1276 as i32 as int16_t,
        1261 as i32 as int16_t,
        1275 as i32 as int16_t,
        1215 as i32 as int16_t,
        1260 as i32 as int16_t,
        1229 as i32 as int16_t,
        -(959 as i32) as int16_t,
        974 as i32 as int16_t,
        974 as i32 as int16_t,
        989 as i32 as int16_t,
        989 as i32 as int16_t,
        -(943 as i32) as int16_t,
        735 as i32 as int16_t,
        478 as i32 as int16_t,
        478 as i32 as int16_t,
        495 as i32 as int16_t,
        463 as i32 as int16_t,
        506 as i32 as int16_t,
        414 as i32 as int16_t,
        -(1039 as i32) as int16_t,
        1003 as i32 as int16_t,
        958 as i32 as int16_t,
        1017 as i32 as int16_t,
        927 as i32 as int16_t,
        942 as i32 as int16_t,
        987 as i32 as int16_t,
        957 as i32 as int16_t,
        431 as i32 as int16_t,
        476 as i32 as int16_t,
        1272 as i32 as int16_t,
        1167 as i32 as int16_t,
        1228 as i32 as int16_t,
        -(1183 as i32) as int16_t,
        1256 as i32 as int16_t,
        -(1199 as i32) as int16_t,
        895 as i32 as int16_t,
        895 as i32 as int16_t,
        941 as i32 as int16_t,
        941 as i32 as int16_t,
        1242 as i32 as int16_t,
        1227 as i32 as int16_t,
        1212 as i32 as int16_t,
        1135 as i32 as int16_t,
        1014 as i32 as int16_t,
        1014 as i32 as int16_t,
        490 as i32 as int16_t,
        489 as i32 as int16_t,
        503 as i32 as int16_t,
        487 as i32 as int16_t,
        910 as i32 as int16_t,
        1013 as i32 as int16_t,
        985 as i32 as int16_t,
        925 as i32 as int16_t,
        863 as i32 as int16_t,
        894 as i32 as int16_t,
        970 as i32 as int16_t,
        955 as i32 as int16_t,
        1012 as i32 as int16_t,
        847 as i32 as int16_t,
        -(1343 as i32) as int16_t,
        831 as i32 as int16_t,
        755 as i32 as int16_t,
        755 as i32 as int16_t,
        984 as i32 as int16_t,
        909 as i32 as int16_t,
        428 as i32 as int16_t,
        366 as i32 as int16_t,
        754 as i32 as int16_t,
        559 as i32 as int16_t,
        -(1391 as i32) as int16_t,
        752 as i32 as int16_t,
        486 as i32 as int16_t,
        457 as i32 as int16_t,
        924 as i32 as int16_t,
        997 as i32 as int16_t,
        698 as i32 as int16_t,
        698 as i32 as int16_t,
        983 as i32 as int16_t,
        893 as i32 as int16_t,
        740 as i32 as int16_t,
        740 as i32 as int16_t,
        908 as i32 as int16_t,
        877 as i32 as int16_t,
        739 as i32 as int16_t,
        739 as i32 as int16_t,
        667 as i32 as int16_t,
        667 as i32 as int16_t,
        953 as i32 as int16_t,
        938 as i32 as int16_t,
        497 as i32 as int16_t,
        287 as i32 as int16_t,
        271 as i32 as int16_t,
        271 as i32 as int16_t,
        683 as i32 as int16_t,
        606 as i32 as int16_t,
        590 as i32 as int16_t,
        712 as i32 as int16_t,
        726 as i32 as int16_t,
        574 as i32 as int16_t,
        302 as i32 as int16_t,
        302 as i32 as int16_t,
        738 as i32 as int16_t,
        736 as i32 as int16_t,
        481 as i32 as int16_t,
        286 as i32 as int16_t,
        526 as i32 as int16_t,
        725 as i32 as int16_t,
        605 as i32 as int16_t,
        711 as i32 as int16_t,
        636 as i32 as int16_t,
        724 as i32 as int16_t,
        696 as i32 as int16_t,
        651 as i32 as int16_t,
        589 as i32 as int16_t,
        681 as i32 as int16_t,
        666 as i32 as int16_t,
        710 as i32 as int16_t,
        364 as i32 as int16_t,
        467 as i32 as int16_t,
        573 as i32 as int16_t,
        695 as i32 as int16_t,
        466 as i32 as int16_t,
        466 as i32 as int16_t,
        301 as i32 as int16_t,
        465 as i32 as int16_t,
        379 as i32 as int16_t,
        379 as i32 as int16_t,
        709 as i32 as int16_t,
        604 as i32 as int16_t,
        665 as i32 as int16_t,
        679 as i32 as int16_t,
        316 as i32 as int16_t,
        316 as i32 as int16_t,
        634 as i32 as int16_t,
        633 as i32 as int16_t,
        436 as i32 as int16_t,
        436 as i32 as int16_t,
        464 as i32 as int16_t,
        269 as i32 as int16_t,
        424 as i32 as int16_t,
        394 as i32 as int16_t,
        452 as i32 as int16_t,
        332 as i32 as int16_t,
        438 as i32 as int16_t,
        363 as i32 as int16_t,
        347 as i32 as int16_t,
        408 as i32 as int16_t,
        393 as i32 as int16_t,
        448 as i32 as int16_t,
        331 as i32 as int16_t,
        422 as i32 as int16_t,
        362 as i32 as int16_t,
        407 as i32 as int16_t,
        392 as i32 as int16_t,
        421 as i32 as int16_t,
        346 as i32 as int16_t,
        406 as i32 as int16_t,
        391 as i32 as int16_t,
        376 as i32 as int16_t,
        375 as i32 as int16_t,
        359 as i32 as int16_t,
        1441 as i32 as int16_t,
        1306 as i32 as int16_t,
        -(2367 as i32) as int16_t,
        1290 as i32 as int16_t,
        -(2383 as i32) as int16_t,
        1337 as i32 as int16_t,
        -(2399 as i32) as int16_t,
        -(2415 as i32) as int16_t,
        1426 as i32 as int16_t,
        1321 as i32 as int16_t,
        -(2431 as i32) as int16_t,
        1411 as i32 as int16_t,
        1336 as i32 as int16_t,
        -(2447 as i32) as int16_t,
        -(2463 as i32) as int16_t,
        -(2479 as i32) as int16_t,
        1169 as i32 as int16_t,
        1169 as i32 as int16_t,
        1049 as i32 as int16_t,
        1049 as i32 as int16_t,
        1424 as i32 as int16_t,
        1289 as i32 as int16_t,
        1412 as i32 as int16_t,
        1352 as i32 as int16_t,
        1319 as i32 as int16_t,
        -(2495 as i32) as int16_t,
        1154 as i32 as int16_t,
        1154 as i32 as int16_t,
        1064 as i32 as int16_t,
        1064 as i32 as int16_t,
        1153 as i32 as int16_t,
        1153 as i32 as int16_t,
        416 as i32 as int16_t,
        390 as i32 as int16_t,
        360 as i32 as int16_t,
        404 as i32 as int16_t,
        403 as i32 as int16_t,
        389 as i32 as int16_t,
        344 as i32 as int16_t,
        374 as i32 as int16_t,
        373 as i32 as int16_t,
        343 as i32 as int16_t,
        358 as i32 as int16_t,
        372 as i32 as int16_t,
        327 as i32 as int16_t,
        357 as i32 as int16_t,
        342 as i32 as int16_t,
        311 as i32 as int16_t,
        356 as i32 as int16_t,
        326 as i32 as int16_t,
        1395 as i32 as int16_t,
        1394 as i32 as int16_t,
        1137 as i32 as int16_t,
        1137 as i32 as int16_t,
        1047 as i32 as int16_t,
        1047 as i32 as int16_t,
        1365 as i32 as int16_t,
        1392 as i32 as int16_t,
        1287 as i32 as int16_t,
        1379 as i32 as int16_t,
        1334 as i32 as int16_t,
        1364 as i32 as int16_t,
        1349 as i32 as int16_t,
        1378 as i32 as int16_t,
        1318 as i32 as int16_t,
        1363 as i32 as int16_t,
        792 as i32 as int16_t,
        792 as i32 as int16_t,
        792 as i32 as int16_t,
        792 as i32 as int16_t,
        1152 as i32 as int16_t,
        1152 as i32 as int16_t,
        1032 as i32 as int16_t,
        1032 as i32 as int16_t,
        1121 as i32 as int16_t,
        1121 as i32 as int16_t,
        1046 as i32 as int16_t,
        1046 as i32 as int16_t,
        1120 as i32 as int16_t,
        1120 as i32 as int16_t,
        1030 as i32 as int16_t,
        1030 as i32 as int16_t,
        -(2895 as i32) as int16_t,
        1106 as i32 as int16_t,
        1061 as i32 as int16_t,
        1104 as i32 as int16_t,
        849 as i32 as int16_t,
        849 as i32 as int16_t,
        789 as i32 as int16_t,
        789 as i32 as int16_t,
        1091 as i32 as int16_t,
        1076 as i32 as int16_t,
        1029 as i32 as int16_t,
        1090 as i32 as int16_t,
        1060 as i32 as int16_t,
        1075 as i32 as int16_t,
        833 as i32 as int16_t,
        833 as i32 as int16_t,
        309 as i32 as int16_t,
        324 as i32 as int16_t,
        532 as i32 as int16_t,
        532 as i32 as int16_t,
        832 as i32 as int16_t,
        772 as i32 as int16_t,
        818 as i32 as int16_t,
        803 as i32 as int16_t,
        561 as i32 as int16_t,
        561 as i32 as int16_t,
        531 as i32 as int16_t,
        560 as i32 as int16_t,
        515 as i32 as int16_t,
        546 as i32 as int16_t,
        289 as i32 as int16_t,
        274 as i32 as int16_t,
        288 as i32 as int16_t,
        258 as i32 as int16_t,
        -(250 as i32) as int16_t,
        -(1179 as i32) as int16_t,
        -(1579 as i32) as int16_t,
        -(1836 as i32) as int16_t,
        -(1996 as i32) as int16_t,
        -(2124 as i32) as int16_t,
        -(2253 as i32) as int16_t,
        -(2333 as i32) as int16_t,
        -(2413 as i32) as int16_t,
        -(2477 as i32) as int16_t,
        -(2542 as i32) as int16_t,
        -(2574 as i32) as int16_t,
        -(2607 as i32) as int16_t,
        -(2622 as i32) as int16_t,
        -(2655 as i32) as int16_t,
        1314 as i32 as int16_t,
        1313 as i32 as int16_t,
        1298 as i32 as int16_t,
        1312 as i32 as int16_t,
        1282 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        785 as i32 as int16_t,
        1040 as i32 as int16_t,
        1040 as i32 as int16_t,
        1025 as i32 as int16_t,
        1025 as i32 as int16_t,
        768 as i32 as int16_t,
        768 as i32 as int16_t,
        768 as i32 as int16_t,
        768 as i32 as int16_t,
        -(766 as i32) as int16_t,
        -(798 as i32) as int16_t,
        -(830 as i32) as int16_t,
        -(862 as i32) as int16_t,
        -(895 as i32) as int16_t,
        -(911 as i32) as int16_t,
        -(927 as i32) as int16_t,
        -(943 as i32) as int16_t,
        -(959 as i32) as int16_t,
        -(975 as i32) as int16_t,
        -(991 as i32) as int16_t,
        -(1007 as i32) as int16_t,
        -(1023 as i32) as int16_t,
        -(1039 as i32) as int16_t,
        -(1055 as i32) as int16_t,
        -(1070 as i32) as int16_t,
        1724 as i32 as int16_t,
        1647 as i32 as int16_t,
        -(1103 as i32) as int16_t,
        -(1119 as i32) as int16_t,
        1631 as i32 as int16_t,
        1767 as i32 as int16_t,
        1662 as i32 as int16_t,
        1738 as i32 as int16_t,
        1708 as i32 as int16_t,
        1723 as i32 as int16_t,
        -(1135 as i32) as int16_t,
        1780 as i32 as int16_t,
        1615 as i32 as int16_t,
        1779 as i32 as int16_t,
        1599 as i32 as int16_t,
        1677 as i32 as int16_t,
        1646 as i32 as int16_t,
        1778 as i32 as int16_t,
        1583 as i32 as int16_t,
        -(1151 as i32) as int16_t,
        1777 as i32 as int16_t,
        1567 as i32 as int16_t,
        1737 as i32 as int16_t,
        1692 as i32 as int16_t,
        1765 as i32 as int16_t,
        1722 as i32 as int16_t,
        1707 as i32 as int16_t,
        1630 as i32 as int16_t,
        1751 as i32 as int16_t,
        1661 as i32 as int16_t,
        1764 as i32 as int16_t,
        1614 as i32 as int16_t,
        1736 as i32 as int16_t,
        1676 as i32 as int16_t,
        1763 as i32 as int16_t,
        1750 as i32 as int16_t,
        1645 as i32 as int16_t,
        1598 as i32 as int16_t,
        1721 as i32 as int16_t,
        1691 as i32 as int16_t,
        1762 as i32 as int16_t,
        1706 as i32 as int16_t,
        1582 as i32 as int16_t,
        1761 as i32 as int16_t,
        1566 as i32 as int16_t,
        -(1167 as i32) as int16_t,
        1749 as i32 as int16_t,
        1629 as i32 as int16_t,
        767 as i32 as int16_t,
        766 as i32 as int16_t,
        751 as i32 as int16_t,
        765 as i32 as int16_t,
        494 as i32 as int16_t,
        494 as i32 as int16_t,
        735 as i32 as int16_t,
        764 as i32 as int16_t,
        719 as i32 as int16_t,
        749 as i32 as int16_t,
        734 as i32 as int16_t,
        763 as i32 as int16_t,
        447 as i32 as int16_t,
        447 as i32 as int16_t,
        748 as i32 as int16_t,
        718 as i32 as int16_t,
        477 as i32 as int16_t,
        506 as i32 as int16_t,
        431 as i32 as int16_t,
        491 as i32 as int16_t,
        446 as i32 as int16_t,
        476 as i32 as int16_t,
        461 as i32 as int16_t,
        505 as i32 as int16_t,
        415 as i32 as int16_t,
        430 as i32 as int16_t,
        475 as i32 as int16_t,
        445 as i32 as int16_t,
        504 as i32 as int16_t,
        399 as i32 as int16_t,
        460 as i32 as int16_t,
        489 as i32 as int16_t,
        414 as i32 as int16_t,
        503 as i32 as int16_t,
        383 as i32 as int16_t,
        474 as i32 as int16_t,
        429 as i32 as int16_t,
        459 as i32 as int16_t,
        502 as i32 as int16_t,
        502 as i32 as int16_t,
        746 as i32 as int16_t,
        752 as i32 as int16_t,
        488 as i32 as int16_t,
        398 as i32 as int16_t,
        501 as i32 as int16_t,
        473 as i32 as int16_t,
        413 as i32 as int16_t,
        472 as i32 as int16_t,
        486 as i32 as int16_t,
        271 as i32 as int16_t,
        480 as i32 as int16_t,
        270 as i32 as int16_t,
        -(1439 as i32) as int16_t,
        -(1455 as i32) as int16_t,
        1357 as i32 as int16_t,
        -(1471 as i32) as int16_t,
        -(1487 as i32) as int16_t,
        -(1503 as i32) as int16_t,
        1341 as i32 as int16_t,
        1325 as i32 as int16_t,
        -(1519 as i32) as int16_t,
        1489 as i32 as int16_t,
        1463 as i32 as int16_t,
        1403 as i32 as int16_t,
        1309 as i32 as int16_t,
        -(1535 as i32) as int16_t,
        1372 as i32 as int16_t,
        1448 as i32 as int16_t,
        1418 as i32 as int16_t,
        1476 as i32 as int16_t,
        1356 as i32 as int16_t,
        1462 as i32 as int16_t,
        1387 as i32 as int16_t,
        -(1551 as i32) as int16_t,
        1475 as i32 as int16_t,
        1340 as i32 as int16_t,
        1447 as i32 as int16_t,
        1402 as i32 as int16_t,
        1386 as i32 as int16_t,
        -(1567 as i32) as int16_t,
        1068 as i32 as int16_t,
        1068 as i32 as int16_t,
        1474 as i32 as int16_t,
        1461 as i32 as int16_t,
        455 as i32 as int16_t,
        380 as i32 as int16_t,
        468 as i32 as int16_t,
        440 as i32 as int16_t,
        395 as i32 as int16_t,
        425 as i32 as int16_t,
        410 as i32 as int16_t,
        454 as i32 as int16_t,
        364 as i32 as int16_t,
        467 as i32 as int16_t,
        466 as i32 as int16_t,
        464 as i32 as int16_t,
        453 as i32 as int16_t,
        269 as i32 as int16_t,
        409 as i32 as int16_t,
        448 as i32 as int16_t,
        268 as i32 as int16_t,
        432 as i32 as int16_t,
        1371 as i32 as int16_t,
        1473 as i32 as int16_t,
        1432 as i32 as int16_t,
        1417 as i32 as int16_t,
        1308 as i32 as int16_t,
        1460 as i32 as int16_t,
        1355 as i32 as int16_t,
        1446 as i32 as int16_t,
        1459 as i32 as int16_t,
        1431 as i32 as int16_t,
        1083 as i32 as int16_t,
        1083 as i32 as int16_t,
        1401 as i32 as int16_t,
        1416 as i32 as int16_t,
        1458 as i32 as int16_t,
        1445 as i32 as int16_t,
        1067 as i32 as int16_t,
        1067 as i32 as int16_t,
        1370 as i32 as int16_t,
        1457 as i32 as int16_t,
        1051 as i32 as int16_t,
        1051 as i32 as int16_t,
        1291 as i32 as int16_t,
        1430 as i32 as int16_t,
        1385 as i32 as int16_t,
        1444 as i32 as int16_t,
        1354 as i32 as int16_t,
        1415 as i32 as int16_t,
        1400 as i32 as int16_t,
        1443 as i32 as int16_t,
        1082 as i32 as int16_t,
        1082 as i32 as int16_t,
        1173 as i32 as int16_t,
        1113 as i32 as int16_t,
        1186 as i32 as int16_t,
        1066 as i32 as int16_t,
        1185 as i32 as int16_t,
        1050 as i32 as int16_t,
        -(1967 as i32) as int16_t,
        1158 as i32 as int16_t,
        1128 as i32 as int16_t,
        1172 as i32 as int16_t,
        1097 as i32 as int16_t,
        1171 as i32 as int16_t,
        1081 as i32 as int16_t,
        -(1983 as i32) as int16_t,
        1157 as i32 as int16_t,
        1112 as i32 as int16_t,
        416 as i32 as int16_t,
        266 as i32 as int16_t,
        375 as i32 as int16_t,
        400 as i32 as int16_t,
        1170 as i32 as int16_t,
        1142 as i32 as int16_t,
        1127 as i32 as int16_t,
        1065 as i32 as int16_t,
        793 as i32 as int16_t,
        793 as i32 as int16_t,
        1169 as i32 as int16_t,
        1033 as i32 as int16_t,
        1156 as i32 as int16_t,
        1096 as i32 as int16_t,
        1141 as i32 as int16_t,
        1111 as i32 as int16_t,
        1155 as i32 as int16_t,
        1080 as i32 as int16_t,
        1126 as i32 as int16_t,
        1140 as i32 as int16_t,
        898 as i32 as int16_t,
        898 as i32 as int16_t,
        808 as i32 as int16_t,
        808 as i32 as int16_t,
        897 as i32 as int16_t,
        897 as i32 as int16_t,
        792 as i32 as int16_t,
        792 as i32 as int16_t,
        1095 as i32 as int16_t,
        1152 as i32 as int16_t,
        1032 as i32 as int16_t,
        1125 as i32 as int16_t,
        1110 as i32 as int16_t,
        1139 as i32 as int16_t,
        1079 as i32 as int16_t,
        1124 as i32 as int16_t,
        882 as i32 as int16_t,
        807 as i32 as int16_t,
        838 as i32 as int16_t,
        881 as i32 as int16_t,
        853 as i32 as int16_t,
        791 as i32 as int16_t,
        -(2319 as i32) as int16_t,
        867 as i32 as int16_t,
        368 as i32 as int16_t,
        263 as i32 as int16_t,
        822 as i32 as int16_t,
        852 as i32 as int16_t,
        837 as i32 as int16_t,
        866 as i32 as int16_t,
        806 as i32 as int16_t,
        865 as i32 as int16_t,
        -(2399 as i32) as int16_t,
        851 as i32 as int16_t,
        352 as i32 as int16_t,
        262 as i32 as int16_t,
        534 as i32 as int16_t,
        534 as i32 as int16_t,
        821 as i32 as int16_t,
        836 as i32 as int16_t,
        594 as i32 as int16_t,
        594 as i32 as int16_t,
        549 as i32 as int16_t,
        549 as i32 as int16_t,
        593 as i32 as int16_t,
        593 as i32 as int16_t,
        533 as i32 as int16_t,
        533 as i32 as int16_t,
        848 as i32 as int16_t,
        773 as i32 as int16_t,
        579 as i32 as int16_t,
        579 as i32 as int16_t,
        564 as i32 as int16_t,
        578 as i32 as int16_t,
        548 as i32 as int16_t,
        563 as i32 as int16_t,
        276 as i32 as int16_t,
        276 as i32 as int16_t,
        577 as i32 as int16_t,
        576 as i32 as int16_t,
        306 as i32 as int16_t,
        291 as i32 as int16_t,
        516 as i32 as int16_t,
        560 as i32 as int16_t,
        305 as i32 as int16_t,
        305 as i32 as int16_t,
        275 as i32 as int16_t,
        259 as i32 as int16_t,
        -(251 as i32) as int16_t,
        -(892 as i32) as int16_t,
        -(2058 as i32) as int16_t,
        -(2620 as i32) as int16_t,
        -(2828 as i32) as int16_t,
        -(2957 as i32) as int16_t,
        -(3023 as i32) as int16_t,
        -(3039 as i32) as int16_t,
        1041 as i32 as int16_t,
        1041 as i32 as int16_t,
        1040 as i32 as int16_t,
        1040 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        769 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        256 as i32 as int16_t,
        -(511 as i32) as int16_t,
        -(527 as i32) as int16_t,
        -(543 as i32) as int16_t,
        -(559 as i32) as int16_t,
        1530 as i32 as int16_t,
        -(575 as i32) as int16_t,
        -(591 as i32) as int16_t,
        1528 as i32 as int16_t,
        1527 as i32 as int16_t,
        1407 as i32 as int16_t,
        1526 as i32 as int16_t,
        1391 as i32 as int16_t,
        1023 as i32 as int16_t,
        1023 as i32 as int16_t,
        1023 as i32 as int16_t,
        1023 as i32 as int16_t,
        1525 as i32 as int16_t,
        1375 as i32 as int16_t,
        1268 as i32 as int16_t,
        1268 as i32 as int16_t,
        1103 as i32 as int16_t,
        1103 as i32 as int16_t,
        1087 as i32 as int16_t,
        1087 as i32 as int16_t,
        1039 as i32 as int16_t,
        1039 as i32 as int16_t,
        1523 as i32 as int16_t,
        -(604 as i32) as int16_t,
        815 as i32 as int16_t,
        815 as i32 as int16_t,
        815 as i32 as int16_t,
        815 as i32 as int16_t,
        510 as i32 as int16_t,
        495 as i32 as int16_t,
        509 as i32 as int16_t,
        479 as i32 as int16_t,
        508 as i32 as int16_t,
        463 as i32 as int16_t,
        507 as i32 as int16_t,
        447 as i32 as int16_t,
        431 as i32 as int16_t,
        505 as i32 as int16_t,
        415 as i32 as int16_t,
        399 as i32 as int16_t,
        -(734 as i32) as int16_t,
        -(782 as i32) as int16_t,
        1262 as i32 as int16_t,
        -(815 as i32) as int16_t,
        1259 as i32 as int16_t,
        1244 as i32 as int16_t,
        -(831 as i32) as int16_t,
        1258 as i32 as int16_t,
        1228 as i32 as int16_t,
        -(847 as i32) as int16_t,
        -(863 as i32) as int16_t,
        1196 as i32 as int16_t,
        -(879 as i32) as int16_t,
        1253 as i32 as int16_t,
        987 as i32 as int16_t,
        987 as i32 as int16_t,
        748 as i32 as int16_t,
        -(767 as i32) as int16_t,
        493 as i32 as int16_t,
        493 as i32 as int16_t,
        462 as i32 as int16_t,
        477 as i32 as int16_t,
        414 as i32 as int16_t,
        414 as i32 as int16_t,
        686 as i32 as int16_t,
        669 as i32 as int16_t,
        478 as i32 as int16_t,
        446 as i32 as int16_t,
        461 as i32 as int16_t,
        445 as i32 as int16_t,
        474 as i32 as int16_t,
        429 as i32 as int16_t,
        487 as i32 as int16_t,
        458 as i32 as int16_t,
        412 as i32 as int16_t,
        471 as i32 as int16_t,
        1266 as i32 as int16_t,
        1264 as i32 as int16_t,
        1009 as i32 as int16_t,
        1009 as i32 as int16_t,
        799 as i32 as int16_t,
        799 as i32 as int16_t,
        -(1019 as i32) as int16_t,
        -(1276 as i32) as int16_t,
        -(1452 as i32) as int16_t,
        -(1581 as i32) as int16_t,
        -(1677 as i32) as int16_t,
        -(1757 as i32) as int16_t,
        -(1821 as i32) as int16_t,
        -(1886 as i32) as int16_t,
        -(1933 as i32) as int16_t,
        -(1997 as i32) as int16_t,
        1257 as i32 as int16_t,
        1257 as i32 as int16_t,
        1483 as i32 as int16_t,
        1468 as i32 as int16_t,
        1512 as i32 as int16_t,
        1422 as i32 as int16_t,
        1497 as i32 as int16_t,
        1406 as i32 as int16_t,
        1467 as i32 as int16_t,
        1496 as i32 as int16_t,
        1421 as i32 as int16_t,
        1510 as i32 as int16_t,
        1134 as i32 as int16_t,
        1134 as i32 as int16_t,
        1225 as i32 as int16_t,
        1225 as i32 as int16_t,
        1466 as i32 as int16_t,
        1451 as i32 as int16_t,
        1374 as i32 as int16_t,
        1405 as i32 as int16_t,
        1252 as i32 as int16_t,
        1252 as i32 as int16_t,
        1358 as i32 as int16_t,
        1480 as i32 as int16_t,
        1164 as i32 as int16_t,
        1164 as i32 as int16_t,
        1251 as i32 as int16_t,
        1251 as i32 as int16_t,
        1238 as i32 as int16_t,
        1238 as i32 as int16_t,
        1389 as i32 as int16_t,
        1465 as i32 as int16_t,
        -(1407 as i32) as int16_t,
        1054 as i32 as int16_t,
        1101 as i32 as int16_t,
        -(1423 as i32) as int16_t,
        1207 as i32 as int16_t,
        -(1439 as i32) as int16_t,
        830 as i32 as int16_t,
        830 as i32 as int16_t,
        1248 as i32 as int16_t,
        1038 as i32 as int16_t,
        1237 as i32 as int16_t,
        1117 as i32 as int16_t,
        1223 as i32 as int16_t,
        1148 as i32 as int16_t,
        1236 as i32 as int16_t,
        1208 as i32 as int16_t,
        411 as i32 as int16_t,
        426 as i32 as int16_t,
        395 as i32 as int16_t,
        410 as i32 as int16_t,
        379 as i32 as int16_t,
        269 as i32 as int16_t,
        1193 as i32 as int16_t,
        1222 as i32 as int16_t,
        1132 as i32 as int16_t,
        1235 as i32 as int16_t,
        1221 as i32 as int16_t,
        1116 as i32 as int16_t,
        976 as i32 as int16_t,
        976 as i32 as int16_t,
        1192 as i32 as int16_t,
        1162 as i32 as int16_t,
        1177 as i32 as int16_t,
        1220 as i32 as int16_t,
        1131 as i32 as int16_t,
        1191 as i32 as int16_t,
        963 as i32 as int16_t,
        963 as i32 as int16_t,
        -(1647 as i32) as int16_t,
        961 as i32 as int16_t,
        780 as i32 as int16_t,
        -(1663 as i32) as int16_t,
        558 as i32 as int16_t,
        558 as i32 as int16_t,
        994 as i32 as int16_t,
        993 as i32 as int16_t,
        437 as i32 as int16_t,
        408 as i32 as int16_t,
        393 as i32 as int16_t,
        407 as i32 as int16_t,
        829 as i32 as int16_t,
        978 as i32 as int16_t,
        813 as i32 as int16_t,
        797 as i32 as int16_t,
        947 as i32 as int16_t,
        -(1743 as i32) as int16_t,
        721 as i32 as int16_t,
        721 as i32 as int16_t,
        377 as i32 as int16_t,
        392 as i32 as int16_t,
        844 as i32 as int16_t,
        950 as i32 as int16_t,
        828 as i32 as int16_t,
        890 as i32 as int16_t,
        706 as i32 as int16_t,
        706 as i32 as int16_t,
        812 as i32 as int16_t,
        859 as i32 as int16_t,
        796 as i32 as int16_t,
        960 as i32 as int16_t,
        948 as i32 as int16_t,
        843 as i32 as int16_t,
        934 as i32 as int16_t,
        874 as i32 as int16_t,
        571 as i32 as int16_t,
        571 as i32 as int16_t,
        -(1919 as i32) as int16_t,
        690 as i32 as int16_t,
        555 as i32 as int16_t,
        689 as i32 as int16_t,
        421 as i32 as int16_t,
        346 as i32 as int16_t,
        539 as i32 as int16_t,
        539 as i32 as int16_t,
        944 as i32 as int16_t,
        779 as i32 as int16_t,
        918 as i32 as int16_t,
        873 as i32 as int16_t,
        932 as i32 as int16_t,
        842 as i32 as int16_t,
        903 as i32 as int16_t,
        888 as i32 as int16_t,
        570 as i32 as int16_t,
        570 as i32 as int16_t,
        931 as i32 as int16_t,
        917 as i32 as int16_t,
        674 as i32 as int16_t,
        674 as i32 as int16_t,
        -(2575 as i32) as int16_t,
        1562 as i32 as int16_t,
        -(2591 as i32) as int16_t,
        1609 as i32 as int16_t,
        -(2607 as i32) as int16_t,
        1654 as i32 as int16_t,
        1322 as i32 as int16_t,
        1322 as i32 as int16_t,
        1441 as i32 as int16_t,
        1441 as i32 as int16_t,
        1696 as i32 as int16_t,
        1546 as i32 as int16_t,
        1683 as i32 as int16_t,
        1593 as i32 as int16_t,
        1669 as i32 as int16_t,
        1624 as i32 as int16_t,
        1426 as i32 as int16_t,
        1426 as i32 as int16_t,
        1321 as i32 as int16_t,
        1321 as i32 as int16_t,
        1639 as i32 as int16_t,
        1680 as i32 as int16_t,
        1425 as i32 as int16_t,
        1425 as i32 as int16_t,
        1305 as i32 as int16_t,
        1305 as i32 as int16_t,
        1545 as i32 as int16_t,
        1668 as i32 as int16_t,
        1608 as i32 as int16_t,
        1623 as i32 as int16_t,
        1667 as i32 as int16_t,
        1592 as i32 as int16_t,
        1638 as i32 as int16_t,
        1666 as i32 as int16_t,
        1320 as i32 as int16_t,
        1320 as i32 as int16_t,
        1652 as i32 as int16_t,
        1607 as i32 as int16_t,
        1409 as i32 as int16_t,
        1409 as i32 as int16_t,
        1304 as i32 as int16_t,
        1304 as i32 as int16_t,
        1288 as i32 as int16_t,
        1288 as i32 as int16_t,
        1664 as i32 as int16_t,
        1637 as i32 as int16_t,
        1395 as i32 as int16_t,
        1395 as i32 as int16_t,
        1335 as i32 as int16_t,
        1335 as i32 as int16_t,
        1622 as i32 as int16_t,
        1636 as i32 as int16_t,
        1394 as i32 as int16_t,
        1394 as i32 as int16_t,
        1319 as i32 as int16_t,
        1319 as i32 as int16_t,
        1606 as i32 as int16_t,
        1621 as i32 as int16_t,
        1392 as i32 as int16_t,
        1392 as i32 as int16_t,
        1137 as i32 as int16_t,
        1137 as i32 as int16_t,
        1137 as i32 as int16_t,
        1137 as i32 as int16_t,
        345 as i32 as int16_t,
        390 as i32 as int16_t,
        360 as i32 as int16_t,
        375 as i32 as int16_t,
        404 as i32 as int16_t,
        373 as i32 as int16_t,
        1047 as i32 as int16_t,
        -(2751 as i32) as int16_t,
        -(2767 as i32) as int16_t,
        -(2783 as i32) as int16_t,
        1062 as i32 as int16_t,
        1121 as i32 as int16_t,
        1046 as i32 as int16_t,
        -(2799 as i32) as int16_t,
        1077 as i32 as int16_t,
        -(2815 as i32) as int16_t,
        1106 as i32 as int16_t,
        1061 as i32 as int16_t,
        789 as i32 as int16_t,
        789 as i32 as int16_t,
        1105 as i32 as int16_t,
        1104 as i32 as int16_t,
        263 as i32 as int16_t,
        355 as i32 as int16_t,
        310 as i32 as int16_t,
        340 as i32 as int16_t,
        325 as i32 as int16_t,
        354 as i32 as int16_t,
        352 as i32 as int16_t,
        262 as i32 as int16_t,
        339 as i32 as int16_t,
        324 as i32 as int16_t,
        1091 as i32 as int16_t,
        1076 as i32 as int16_t,
        1029 as i32 as int16_t,
        1090 as i32 as int16_t,
        1060 as i32 as int16_t,
        1075 as i32 as int16_t,
        833 as i32 as int16_t,
        833 as i32 as int16_t,
        788 as i32 as int16_t,
        788 as i32 as int16_t,
        1088 as i32 as int16_t,
        1028 as i32 as int16_t,
        818 as i32 as int16_t,
        818 as i32 as int16_t,
        803 as i32 as int16_t,
        803 as i32 as int16_t,
        561 as i32 as int16_t,
        561 as i32 as int16_t,
        531 as i32 as int16_t,
        531 as i32 as int16_t,
        816 as i32 as int16_t,
        771 as i32 as int16_t,
        546 as i32 as int16_t,
        546 as i32 as int16_t,
        289 as i32 as int16_t,
        274 as i32 as int16_t,
        288 as i32 as int16_t,
        258 as i32 as int16_t,
        -(253 as i32) as int16_t,
        -(317 as i32) as int16_t,
        -(381 as i32) as int16_t,
        -(446 as i32) as int16_t,
        -(478 as i32) as int16_t,
        -(509 as i32) as int16_t,
        1279 as i32 as int16_t,
        1279 as i32 as int16_t,
        -(811 as i32) as int16_t,
        -(1179 as i32) as int16_t,
        -(1451 as i32) as int16_t,
        -(1756 as i32) as int16_t,
        -(1900 as i32) as int16_t,
        -(2028 as i32) as int16_t,
        -(2189 as i32) as int16_t,
        -(2253 as i32) as int16_t,
        -(2333 as i32) as int16_t,
        -(2414 as i32) as int16_t,
        -(2445 as i32) as int16_t,
        -(2511 as i32) as int16_t,
        -(2526 as i32) as int16_t,
        1313 as i32 as int16_t,
        1298 as i32 as int16_t,
        -(2559 as i32) as int16_t,
        1041 as i32 as int16_t,
        1041 as i32 as int16_t,
        1040 as i32 as int16_t,
        1040 as i32 as int16_t,
        1025 as i32 as int16_t,
        1025 as i32 as int16_t,
        1024 as i32 as int16_t,
        1024 as i32 as int16_t,
        1022 as i32 as int16_t,
        1007 as i32 as int16_t,
        1021 as i32 as int16_t,
        991 as i32 as int16_t,
        1020 as i32 as int16_t,
        975 as i32 as int16_t,
        1019 as i32 as int16_t,
        959 as i32 as int16_t,
        687 as i32 as int16_t,
        687 as i32 as int16_t,
        1018 as i32 as int16_t,
        1017 as i32 as int16_t,
        671 as i32 as int16_t,
        671 as i32 as int16_t,
        655 as i32 as int16_t,
        655 as i32 as int16_t,
        1016 as i32 as int16_t,
        1015 as i32 as int16_t,
        639 as i32 as int16_t,
        639 as i32 as int16_t,
        758 as i32 as int16_t,
        758 as i32 as int16_t,
        623 as i32 as int16_t,
        623 as i32 as int16_t,
        757 as i32 as int16_t,
        607 as i32 as int16_t,
        756 as i32 as int16_t,
        591 as i32 as int16_t,
        755 as i32 as int16_t,
        575 as i32 as int16_t,
        754 as i32 as int16_t,
        559 as i32 as int16_t,
        543 as i32 as int16_t,
        543 as i32 as int16_t,
        1009 as i32 as int16_t,
        783 as i32 as int16_t,
        -(575 as i32) as int16_t,
        -(621 as i32) as int16_t,
        -(685 as i32) as int16_t,
        -(749 as i32) as int16_t,
        496 as i32 as int16_t,
        -(590 as i32) as int16_t,
        750 as i32 as int16_t,
        749 as i32 as int16_t,
        734 as i32 as int16_t,
        748 as i32 as int16_t,
        974 as i32 as int16_t,
        989 as i32 as int16_t,
        1003 as i32 as int16_t,
        958 as i32 as int16_t,
        988 as i32 as int16_t,
        973 as i32 as int16_t,
        1002 as i32 as int16_t,
        942 as i32 as int16_t,
        987 as i32 as int16_t,
        957 as i32 as int16_t,
        972 as i32 as int16_t,
        1001 as i32 as int16_t,
        926 as i32 as int16_t,
        986 as i32 as int16_t,
        941 as i32 as int16_t,
        971 as i32 as int16_t,
        956 as i32 as int16_t,
        1000 as i32 as int16_t,
        910 as i32 as int16_t,
        985 as i32 as int16_t,
        925 as i32 as int16_t,
        999 as i32 as int16_t,
        894 as i32 as int16_t,
        970 as i32 as int16_t,
        -(1071 as i32) as int16_t,
        -(1087 as i32) as int16_t,
        -(1102 as i32) as int16_t,
        1390 as i32 as int16_t,
        -(1135 as i32) as int16_t,
        1436 as i32 as int16_t,
        1509 as i32 as int16_t,
        1451 as i32 as int16_t,
        1374 as i32 as int16_t,
        -(1151 as i32) as int16_t,
        1405 as i32 as int16_t,
        1358 as i32 as int16_t,
        1480 as i32 as int16_t,
        1420 as i32 as int16_t,
        -(1167 as i32) as int16_t,
        1507 as i32 as int16_t,
        1494 as i32 as int16_t,
        1389 as i32 as int16_t,
        1342 as i32 as int16_t,
        1465 as i32 as int16_t,
        1435 as i32 as int16_t,
        1450 as i32 as int16_t,
        1326 as i32 as int16_t,
        1505 as i32 as int16_t,
        1310 as i32 as int16_t,
        1493 as i32 as int16_t,
        1373 as i32 as int16_t,
        1479 as i32 as int16_t,
        1404 as i32 as int16_t,
        1492 as i32 as int16_t,
        1464 as i32 as int16_t,
        1419 as i32 as int16_t,
        428 as i32 as int16_t,
        443 as i32 as int16_t,
        472 as i32 as int16_t,
        397 as i32 as int16_t,
        736 as i32 as int16_t,
        526 as i32 as int16_t,
        464 as i32 as int16_t,
        464 as i32 as int16_t,
        486 as i32 as int16_t,
        457 as i32 as int16_t,
        442 as i32 as int16_t,
        471 as i32 as int16_t,
        484 as i32 as int16_t,
        482 as i32 as int16_t,
        1357 as i32 as int16_t,
        1449 as i32 as int16_t,
        1434 as i32 as int16_t,
        1478 as i32 as int16_t,
        1388 as i32 as int16_t,
        1491 as i32 as int16_t,
        1341 as i32 as int16_t,
        1490 as i32 as int16_t,
        1325 as i32 as int16_t,
        1489 as i32 as int16_t,
        1463 as i32 as int16_t,
        1403 as i32 as int16_t,
        1309 as i32 as int16_t,
        1477 as i32 as int16_t,
        1372 as i32 as int16_t,
        1448 as i32 as int16_t,
        1418 as i32 as int16_t,
        1433 as i32 as int16_t,
        1476 as i32 as int16_t,
        1356 as i32 as int16_t,
        1462 as i32 as int16_t,
        1387 as i32 as int16_t,
        -(1439 as i32) as int16_t,
        1475 as i32 as int16_t,
        1340 as i32 as int16_t,
        1447 as i32 as int16_t,
        1402 as i32 as int16_t,
        1474 as i32 as int16_t,
        1324 as i32 as int16_t,
        1461 as i32 as int16_t,
        1371 as i32 as int16_t,
        1473 as i32 as int16_t,
        269 as i32 as int16_t,
        448 as i32 as int16_t,
        1432 as i32 as int16_t,
        1417 as i32 as int16_t,
        1308 as i32 as int16_t,
        1460 as i32 as int16_t,
        -(1711 as i32) as int16_t,
        1459 as i32 as int16_t,
        -(1727 as i32) as int16_t,
        1441 as i32 as int16_t,
        1099 as i32 as int16_t,
        1099 as i32 as int16_t,
        1446 as i32 as int16_t,
        1386 as i32 as int16_t,
        1431 as i32 as int16_t,
        1401 as i32 as int16_t,
        -(1743 as i32) as int16_t,
        1289 as i32 as int16_t,
        1083 as i32 as int16_t,
        1083 as i32 as int16_t,
        1160 as i32 as int16_t,
        1160 as i32 as int16_t,
        1458 as i32 as int16_t,
        1445 as i32 as int16_t,
        1067 as i32 as int16_t,
        1067 as i32 as int16_t,
        1370 as i32 as int16_t,
        1457 as i32 as int16_t,
        1307 as i32 as int16_t,
        1430 as i32 as int16_t,
        1129 as i32 as int16_t,
        1129 as i32 as int16_t,
        1098 as i32 as int16_t,
        1098 as i32 as int16_t,
        268 as i32 as int16_t,
        432 as i32 as int16_t,
        267 as i32 as int16_t,
        416 as i32 as int16_t,
        266 as i32 as int16_t,
        400 as i32 as int16_t,
        -(1887 as i32) as int16_t,
        1144 as i32 as int16_t,
        1187 as i32 as int16_t,
        1082 as i32 as int16_t,
        1173 as i32 as int16_t,
        1113 as i32 as int16_t,
        1186 as i32 as int16_t,
        1066 as i32 as int16_t,
        1050 as i32 as int16_t,
        1158 as i32 as int16_t,
        1128 as i32 as int16_t,
        1143 as i32 as int16_t,
        1172 as i32 as int16_t,
        1097 as i32 as int16_t,
        1171 as i32 as int16_t,
        1081 as i32 as int16_t,
        420 as i32 as int16_t,
        391 as i32 as int16_t,
        1157 as i32 as int16_t,
        1112 as i32 as int16_t,
        1170 as i32 as int16_t,
        1142 as i32 as int16_t,
        1127 as i32 as int16_t,
        1065 as i32 as int16_t,
        1169 as i32 as int16_t,
        1049 as i32 as int16_t,
        1156 as i32 as int16_t,
        1096 as i32 as int16_t,
        1141 as i32 as int16_t,
        1111 as i32 as int16_t,
        1155 as i32 as int16_t,
        1080 as i32 as int16_t,
        1126 as i32 as int16_t,
        1154 as i32 as int16_t,
        1064 as i32 as int16_t,
        1153 as i32 as int16_t,
        1140 as i32 as int16_t,
        1095 as i32 as int16_t,
        1048 as i32 as int16_t,
        -(2159 as i32) as int16_t,
        1125 as i32 as int16_t,
        1110 as i32 as int16_t,
        1137 as i32 as int16_t,
        -(2175 as i32) as int16_t,
        823 as i32 as int16_t,
        823 as i32 as int16_t,
        1139 as i32 as int16_t,
        1138 as i32 as int16_t,
        807 as i32 as int16_t,
        807 as i32 as int16_t,
        384 as i32 as int16_t,
        264 as i32 as int16_t,
        368 as i32 as int16_t,
        263 as i32 as int16_t,
        868 as i32 as int16_t,
        838 as i32 as int16_t,
        853 as i32 as int16_t,
        791 as i32 as int16_t,
        867 as i32 as int16_t,
        822 as i32 as int16_t,
        852 as i32 as int16_t,
        837 as i32 as int16_t,
        866 as i32 as int16_t,
        806 as i32 as int16_t,
        865 as i32 as int16_t,
        790 as i32 as int16_t,
        -(2319 as i32) as int16_t,
        851 as i32 as int16_t,
        821 as i32 as int16_t,
        836 as i32 as int16_t,
        352 as i32 as int16_t,
        262 as i32 as int16_t,
        850 as i32 as int16_t,
        805 as i32 as int16_t,
        849 as i32 as int16_t,
        -(2399 as i32) as int16_t,
        533 as i32 as int16_t,
        533 as i32 as int16_t,
        835 as i32 as int16_t,
        820 as i32 as int16_t,
        336 as i32 as int16_t,
        261 as i32 as int16_t,
        578 as i32 as int16_t,
        548 as i32 as int16_t,
        563 as i32 as int16_t,
        577 as i32 as int16_t,
        532 as i32 as int16_t,
        532 as i32 as int16_t,
        832 as i32 as int16_t,
        772 as i32 as int16_t,
        562 as i32 as int16_t,
        562 as i32 as int16_t,
        547 as i32 as int16_t,
        547 as i32 as int16_t,
        305 as i32 as int16_t,
        275 as i32 as int16_t,
        560 as i32 as int16_t,
        515 as i32 as int16_t,
        290 as i32 as int16_t,
        290 as i32 as int16_t,
        288 as i32 as int16_t,
        258 as i32 as int16_t,
    ];
    static tab32: [uint8_t; 28] = [
        130 as i32 as uint8_t,
        162 as i32 as uint8_t,
        193 as i32 as uint8_t,
        209 as i32 as uint8_t,
        44 as i32 as uint8_t,
        28 as i32 as uint8_t,
        76 as i32 as uint8_t,
        140 as i32 as uint8_t,
        9 as i32 as uint8_t,
        9 as i32 as uint8_t,
        9 as i32 as uint8_t,
        9 as i32 as uint8_t,
        9 as i32 as uint8_t,
        9 as i32 as uint8_t,
        9 as i32 as uint8_t,
        9 as i32 as uint8_t,
        190 as i32 as uint8_t,
        254 as i32 as uint8_t,
        222 as i32 as uint8_t,
        238 as i32 as uint8_t,
        126 as i32 as uint8_t,
        94 as i32 as uint8_t,
        157 as i32 as uint8_t,
        157 as i32 as uint8_t,
        109 as i32 as uint8_t,
        61 as i32 as uint8_t,
        173 as i32 as uint8_t,
        205 as i32 as uint8_t,
    ];
    static tab33: [uint8_t; 16] = [
        252 as i32 as uint8_t,
        236 as i32 as uint8_t,
        220 as i32 as uint8_t,
        204 as i32 as uint8_t,
        188 as i32 as uint8_t,
        172 as i32 as uint8_t,
        156 as i32 as uint8_t,
        140 as i32 as uint8_t,
        124 as i32 as uint8_t,
        108 as i32 as uint8_t,
        92 as i32 as uint8_t,
        76 as i32 as uint8_t,
        60 as i32 as uint8_t,
        44 as i32 as uint8_t,
        28 as i32 as uint8_t,
        12 as i32 as uint8_t,
    ];
    static tabindex: [int16_t; 32] = [
        0 as i32 as int16_t,
        32 as i32 as int16_t,
        64 as i32 as int16_t,
        98 as i32 as int16_t,
        0 as i32 as int16_t,
        132 as i32 as int16_t,
        180 as i32 as int16_t,
        218 as i32 as int16_t,
        292 as i32 as int16_t,
        364 as i32 as int16_t,
        426 as i32 as int16_t,
        538 as i32 as int16_t,
        648 as i32 as int16_t,
        746 as i32 as int16_t,
        0 as i32 as int16_t,
        1126 as i32 as int16_t,
        1460 as i32 as int16_t,
        1460 as i32 as int16_t,
        1460 as i32 as int16_t,
        1460 as i32 as int16_t,
        1460 as i32 as int16_t,
        1460 as i32 as int16_t,
        1460 as i32 as int16_t,
        1460 as i32 as int16_t,
        1842 as i32 as int16_t,
        1842 as i32 as int16_t,
        1842 as i32 as int16_t,
        1842 as i32 as int16_t,
        1842 as i32 as int16_t,
        1842 as i32 as int16_t,
        1842 as i32 as int16_t,
        1842 as i32 as int16_t,
    ];
    static g_linbits: [uint8_t; 32] = [
        0 as i32 as uint8_t,
        0 as i32 as uint8_t,
        0 as i32 as uint8_t,
        0 as i32 as uint8_t,
        0 as i32 as uint8_t,
        0 as i32 as uint8_t,
        0 as i32 as uint8_t,
        0 as i32 as uint8_t,
        0 as i32 as uint8_t,
        0 as i32 as uint8_t,
        0 as i32 as uint8_t,
        0 as i32 as uint8_t,
        0 as i32 as uint8_t,
        0 as i32 as uint8_t,
        0 as i32 as uint8_t,
        0 as i32 as uint8_t,
        1 as i32 as uint8_t,
        2 as i32 as uint8_t,
        3 as i32 as uint8_t,
        4 as i32 as uint8_t,
        6 as i32 as uint8_t,
        8 as i32 as uint8_t,
        10 as i32 as uint8_t,
        13 as i32 as uint8_t,
        4 as i32 as uint8_t,
        5 as i32 as uint8_t,
        6 as i32 as uint8_t,
        7 as i32 as uint8_t,
        8 as i32 as uint8_t,
        9 as i32 as uint8_t,
        11 as i32 as uint8_t,
        13 as i32 as uint8_t,
    ];
    let mut one: f32 = 0.0f32;
    let mut ireg: i32 = 0 as i32;
    let mut big_val_cnt: i32 = (*gr_info).big_values as i32;
    let mut sfb: *const uint8_t = (*gr_info).sfbtab;
    let mut bs_next_ptr: *const uint8_t = ((*bs).buf).offset(((*bs).pos / 8 as i32) as isize);
    let mut bs_cache: uint32_t = (*bs_next_ptr.offset(0 as i32 as isize) as u32)
        .wrapping_mul(256 as u32)
        .wrapping_add(*bs_next_ptr.offset(1 as i32 as isize) as u32)
        .wrapping_mul(256 as u32)
        .wrapping_add(*bs_next_ptr.offset(2 as i32 as isize) as u32)
        .wrapping_mul(256 as u32)
        .wrapping_add(*bs_next_ptr.offset(3 as i32 as isize) as u32)
        << ((*bs).pos & 7 as i32);
    let mut pairs_to_decode: i32 = 0;
    let mut np: i32 = 0;
    let mut bs_sh: i32 = ((*bs).pos & 7 as i32) - 8 as i32;
    bs_next_ptr = bs_next_ptr.offset(4 as i32 as isize);
    while big_val_cnt > 0 as i32 {
        let mut tab_num: i32 = (*gr_info).table_select[ireg as usize] as i32;
        let fresh4 = ireg;
        ireg = ireg + 1;
        let mut sfb_cnt: i32 = (*gr_info).region_count[fresh4 as usize] as i32;
        let mut codebook: *const int16_t = tabs
            .as_ptr()
            .offset(tabindex[tab_num as usize] as i32 as isize);
        let mut linbits: i32 = g_linbits[tab_num as usize] as i32;
        if linbits != 0 {
            loop {
                let fresh5 = sfb;
                sfb = sfb.offset(1);
                np = *fresh5 as i32 / 2 as i32;
                pairs_to_decode = if big_val_cnt > np { np } else { big_val_cnt };
                let fresh6 = scf;
                scf = scf.offset(1);
                one = *fresh6;
                loop {
                    let mut j: i32 = 0;
                    let mut w: i32 = 5 as i32;
                    let mut leaf: i32 =
                        *codebook.offset((bs_cache >> 32 as i32 - w) as isize) as i32;
                    while leaf < 0 as i32 {
                        bs_cache <<= w;
                        bs_sh += w;
                        w = leaf & 7 as i32;
                        leaf = *codebook.offset(
                            (bs_cache >> 32 as i32 - w).wrapping_sub((leaf >> 3 as i32) as uint32_t)
                                as isize,
                        ) as i32;
                    }
                    bs_cache <<= leaf >> 8 as i32;
                    bs_sh += leaf >> 8 as i32;
                    j = 0 as i32;
                    while j < 2 as i32 {
                        let mut lsb: i32 = leaf & 0xf as i32;
                        if lsb == 15 as i32 {
                            lsb = (lsb as uint32_t).wrapping_add(bs_cache >> 32 as i32 - linbits)
                                as i32 as i32;
                            bs_cache <<= linbits;
                            bs_sh += linbits;
                            while bs_sh >= 0 as i32 {
                                let fresh7 = bs_next_ptr;
                                bs_next_ptr = bs_next_ptr.offset(1);
                                bs_cache |= (*fresh7 as uint32_t) << bs_sh;
                                bs_sh -= 8 as i32;
                            }
                            *dst = one
                                * L3_pow_43(lsb)
                                * (if (bs_cache as int32_t) < 0 as i32 {
                                    -(1 as i32)
                                } else {
                                    1 as i32
                                }) as f32;
                        } else {
                            *dst = g_pow43[((16 as i32 + lsb) as uint32_t)
                                .wrapping_sub(16 as i32 as uint32_t * (bs_cache >> 31 as i32))
                                as usize]
                                * one;
                        }
                        bs_cache <<= if lsb != 0 { 1 as i32 } else { 0 as i32 };
                        bs_sh += if lsb != 0 { 1 as i32 } else { 0 as i32 };
                        j += 1;
                        dst = dst.offset(1);
                        leaf >>= 4 as i32;
                    }
                    while bs_sh >= 0 as i32 {
                        let fresh8 = bs_next_ptr;
                        bs_next_ptr = bs_next_ptr.offset(1);
                        bs_cache |= (*fresh8 as uint32_t) << bs_sh;
                        bs_sh -= 8 as i32;
                    }
                    pairs_to_decode -= 1;
                    if !(pairs_to_decode != 0) {
                        break;
                    }
                }
                big_val_cnt -= np;
                if !(big_val_cnt > 0 as i32 && {
                    sfb_cnt -= 1;
                    sfb_cnt >= 0 as i32
                }) {
                    break;
                }
            }
        } else {
            loop {
                let fresh9 = sfb;
                sfb = sfb.offset(1);
                np = *fresh9 as i32 / 2 as i32;
                pairs_to_decode = if big_val_cnt > np { np } else { big_val_cnt };
                let fresh10 = scf;
                scf = scf.offset(1);
                one = *fresh10;
                loop {
                    let mut j_0: i32 = 0;
                    let mut w_0: i32 = 5 as i32;
                    let mut leaf_0: i32 =
                        *codebook.offset((bs_cache >> 32 as i32 - w_0) as isize) as i32;
                    while leaf_0 < 0 as i32 {
                        bs_cache <<= w_0;
                        bs_sh += w_0;
                        w_0 = leaf_0 & 7 as i32;
                        leaf_0 = *codebook.offset(
                            (bs_cache >> 32 as i32 - w_0)
                                .wrapping_sub((leaf_0 >> 3 as i32) as uint32_t)
                                as isize,
                        ) as i32;
                    }
                    bs_cache <<= leaf_0 >> 8 as i32;
                    bs_sh += leaf_0 >> 8 as i32;
                    j_0 = 0 as i32;
                    while j_0 < 2 as i32 {
                        let mut lsb_0: i32 = leaf_0 & 0xf as i32;
                        *dst = g_pow43[((16 as i32 + lsb_0) as uint32_t)
                            .wrapping_sub(16 as i32 as uint32_t * (bs_cache >> 31 as i32))
                            as usize]
                            * one;
                        bs_cache <<= if lsb_0 != 0 { 1 as i32 } else { 0 as i32 };
                        bs_sh += if lsb_0 != 0 { 1 as i32 } else { 0 as i32 };
                        j_0 += 1;
                        dst = dst.offset(1);
                        leaf_0 >>= 4 as i32;
                    }
                    while bs_sh >= 0 as i32 {
                        let fresh11 = bs_next_ptr;
                        bs_next_ptr = bs_next_ptr.offset(1);
                        bs_cache |= (*fresh11 as uint32_t) << bs_sh;
                        bs_sh -= 8 as i32;
                    }
                    pairs_to_decode -= 1;
                    if !(pairs_to_decode != 0) {
                        break;
                    }
                }
                big_val_cnt -= np;
                if !(big_val_cnt > 0 as i32 && {
                    sfb_cnt -= 1;
                    sfb_cnt >= 0 as i32
                }) {
                    break;
                }
            }
        }
    }
    np = 1 as i32 - big_val_cnt;
    // A Layer III granule contains at most 576 spectral lines. Keep the
    // translated count1 loop bounded by the space left after the big-values
    // pairs: malformed state can otherwise leave `np` negative while a
    // zero-width Huffman leaf consumes no bits, making this loop permanent.
    let mut count1_quads: i32 = 0;
    let count1_limit: i32 =
        (576 as i32 - 2 as i32 * (*gr_info).big_values as i32).max(0 as i32) / 4 as i32;
    loop {
        if count1_quads >= count1_limit {
            break;
        }
        count1_quads += 1;
        let mut codebook_count1: *const uint8_t = if (*gr_info).count1_table as i32 != 0 {
            tab33.as_ptr()
        } else {
            tab32.as_ptr()
        };
        let mut leaf_1: i32 =
            *codebook_count1.offset((bs_cache >> 32 as i32 - 4 as i32) as isize) as i32;
        if leaf_1 & 8 as i32 == 0 {
            leaf_1 = *codebook_count1.offset(
                ((leaf_1 >> 3 as i32) as uint32_t)
                    .wrapping_add(bs_cache << 4 as i32 >> 32 as i32 - (leaf_1 & 3 as i32))
                    as isize,
            ) as i32;
        }
        bs_cache <<= leaf_1 & 7 as i32;
        bs_sh += leaf_1 & 7 as i32;
        if bs_next_ptr.offset_from((*bs).buf) as isize * 8 as i32 as isize - 24 as i32 as isize
            + bs_sh as isize
            > layer3gr_limit as isize
        {
            break;
        }
        np -= 1;
        if np == 0 {
            let fresh12 = sfb;
            sfb = sfb.offset(1);
            np = *fresh12 as i32 / 2 as i32;
            if np == 0 {
                break;
            }
            let fresh13 = scf;
            scf = scf.offset(1);
            one = *fresh13;
        }
        if leaf_1 & 128 as i32 >> 0 as i32 != 0 {
            *dst.offset(0 as i32 as isize) = if (bs_cache as int32_t) < 0 as i32 {
                -one
            } else {
                one
            };
            bs_cache <<= 1 as i32;
            bs_sh += 1 as i32;
        }
        if leaf_1 & 128 as i32 >> 1 as i32 != 0 {
            *dst.offset(1 as i32 as isize) = if (bs_cache as int32_t) < 0 as i32 {
                -one
            } else {
                one
            };
            bs_cache <<= 1 as i32;
            bs_sh += 1 as i32;
        }
        np -= 1;
        if np == 0 {
            let fresh14 = sfb;
            sfb = sfb.offset(1);
            np = *fresh14 as i32 / 2 as i32;
            if np == 0 {
                break;
            }
            let fresh15 = scf;
            scf = scf.offset(1);
            one = *fresh15;
        }
        if leaf_1 & 128 as i32 >> 2 as i32 != 0 {
            *dst.offset(2 as i32 as isize) = if (bs_cache as int32_t) < 0 as i32 {
                -one
            } else {
                one
            };
            bs_cache <<= 1 as i32;
            bs_sh += 1 as i32;
        }
        if leaf_1 & 128 as i32 >> 3 as i32 != 0 {
            *dst.offset(3 as i32 as isize) = if (bs_cache as int32_t) < 0 as i32 {
                -one
            } else {
                one
            };
            bs_cache <<= 1 as i32;
            bs_sh += 1 as i32;
        }
        while bs_sh >= 0 as i32 {
            let fresh16 = bs_next_ptr;
            bs_next_ptr = bs_next_ptr.offset(1);
            bs_cache |= (*fresh16 as uint32_t) << bs_sh;
            bs_sh -= 8 as i32;
        }
        dst = dst.offset(4 as i32 as isize);
    }
    (*bs).pos = layer3gr_limit;
}
unsafe fn L3_midside_stereo(mut left: *mut f32, mut n: i32) {
    let mut i: i32 = 0 as i32;
    let mut right: *mut f32 = left.offset(576 as i32 as isize);
    while i < n {
        let mut a: f32 = *left.offset(i as isize);
        let mut b: f32 = *right.offset(i as isize);
        *left.offset(i as isize) = a + b;
        *right.offset(i as isize) = a - b;
        i += 1;
    }
}
unsafe fn L3_intensity_stereo_band(mut left: *mut f32, mut n: i32, mut kl: f32, mut kr: f32) {
    let mut i: i32 = 0;
    i = 0 as i32;
    while i < n {
        *left.offset((i + 576 as i32) as isize) = *left.offset(i as isize) * kr;
        *left.offset(i as isize) = *left.offset(i as isize) * kl;
        i += 1;
    }
}
unsafe fn L3_stereo_top_band(
    mut right: *const f32,
    mut sfb: *const uint8_t,
    mut nbands: i32,
    mut max_band: *mut i32,
) {
    let mut i: i32 = 0;
    let mut k: i32 = 0;
    let ref mut fresh17 = *max_band.offset(2 as i32 as isize);
    *fresh17 = -(1 as i32);
    let ref mut fresh18 = *max_band.offset(1 as i32 as isize);
    *fresh18 = *fresh17;
    *max_band.offset(0 as i32 as isize) = *fresh18;
    i = 0 as i32;
    while i < nbands {
        k = 0 as i32;
        while k < *sfb.offset(i as isize) as i32 {
            if *right.offset(k as isize) != 0 as i32 as f32
                || *right.offset((k + 1 as i32) as isize) != 0 as i32 as f32
            {
                *max_band.offset((i % 3 as i32) as isize) = i;
                break;
            } else {
                k += 2 as i32;
            }
        }
        right = right.offset(*sfb.offset(i as isize) as i32 as isize);
        i += 1;
    }
}
unsafe fn L3_stereo_process(
    mut left: *mut f32,
    mut ist_pos: *const uint8_t,
    mut sfb: *const uint8_t,
    mut hdr: *const uint8_t,
    mut max_band: *mut i32,
    mut mpeg2_sh: i32,
) {
    static g_pan: [f32; 14] = [
        0 as i32 as f32,
        1 as i32 as f32,
        0.21132487f32,
        0.78867513f32,
        0.36602540f32,
        0.63397460f32,
        0.5f32,
        0.5f32,
        0.63397460f32,
        0.36602540f32,
        0.78867513f32,
        0.21132487f32,
        1 as i32 as f32,
        0 as i32 as f32,
    ];
    let mut i: u32 = 0;
    let mut max_pos: u32 = (if *hdr.offset(1 as i32 as isize) as i32 & 0x8 as i32 != 0 {
        7 as i32
    } else {
        64 as i32
    }) as u32;
    i = 0 as i32 as u32;
    while *sfb.offset(i as isize) != 0 {
        let mut ipos: u32 = *ist_pos.offset(i as isize) as u32;
        if i as i32 > *max_band.offset(i.wrapping_rem(3 as i32 as u32) as isize) && ipos < max_pos {
            let mut kl: f32 = 0.;
            let mut kr: f32 = 0.;
            let mut s: f32 = if *hdr.offset(3 as i32 as isize) as i32 & 0x20 as i32 != 0 {
                1.41421356f32
            } else {
                1 as i32 as f32
            };
            if *hdr.offset(1 as i32 as isize) as i32 & 0x8 as i32 != 0 {
                kl = g_pan[(2 as i32 as u32).wrapping_mul(ipos) as usize];
                kr = g_pan[(2 as i32 as u32)
                    .wrapping_mul(ipos)
                    .wrapping_add(1 as i32 as u32) as usize];
            } else {
                kl = 1 as i32 as f32;
                kr = L3_ldexp_q2(
                    1 as i32 as f32,
                    ((ipos.wrapping_add(1 as i32 as u32) >> 1 as i32) << mpeg2_sh) as i32,
                );
                if ipos & 1 as i32 as u32 != 0 {
                    kl = kr;
                    kr = 1 as i32 as f32;
                }
            }
            L3_intensity_stereo_band(left, *sfb.offset(i as isize) as i32, kl * s, kr * s);
        } else if *hdr.offset(3 as i32 as isize) as i32 & 0x20 as i32 != 0 {
            L3_midside_stereo(left, *sfb.offset(i as isize) as i32);
        }
        left = left.offset(*sfb.offset(i as isize) as i32 as isize);
        i = i.wrapping_add(1);
    }
}
unsafe fn L3_intensity_stereo(
    mut left: *mut f32,
    mut ist_pos: *mut uint8_t,
    mut gr: *const L3_gr_info_t,
    mut hdr: *const uint8_t,
) {
    let mut max_band: [i32; 3] = [0; 3];
    let mut n_sfb: i32 = (*gr).n_long_sfb as i32 + (*gr).n_short_sfb as i32;
    let mut i: i32 = 0;
    let mut max_blocks: i32 = if (*gr).n_short_sfb as i32 != 0 {
        3 as i32
    } else {
        1 as i32
    };
    L3_stereo_top_band(
        left.offset(576 as i32 as isize),
        (*gr).sfbtab,
        n_sfb,
        max_band.as_mut_ptr(),
    );
    if (*gr).n_long_sfb != 0 {
        max_band[2 as i32 as usize] =
            if (if max_band[0 as i32 as usize] < max_band[1 as i32 as usize] {
                max_band[1 as i32 as usize]
            } else {
                max_band[0 as i32 as usize]
            }) < max_band[2 as i32 as usize]
            {
                max_band[2 as i32 as usize]
            } else if max_band[0 as i32 as usize] < max_band[1 as i32 as usize] {
                max_band[1 as i32 as usize]
            } else {
                max_band[0 as i32 as usize]
            };
        max_band[1 as i32 as usize] = max_band[2 as i32 as usize];
        max_band[0 as i32 as usize] = max_band[1 as i32 as usize];
    }
    i = 0 as i32;
    while i < max_blocks {
        let mut default_pos: i32 = if *hdr.offset(1 as i32 as isize) as i32 & 0x8 as i32 != 0 {
            3 as i32
        } else {
            0 as i32
        };
        let mut itop: i32 = n_sfb - max_blocks + i;
        let mut prev: i32 = itop - max_blocks;
        *ist_pos.offset(itop as isize) = (if max_band[i as usize] >= prev {
            default_pos
        } else {
            *ist_pos.offset(prev as isize) as i32
        }) as uint8_t;
        i += 1;
    }
    L3_stereo_process(
        left,
        ist_pos,
        (*gr).sfbtab,
        hdr,
        max_band.as_mut_ptr(),
        (*gr.offset(1 as i32 as isize)).scalefac_compress as i32 & 1 as i32,
    );
}
unsafe fn L3_reorder(mut grbuf: *mut f32, mut scratch: *mut f32, mut sfb: *const uint8_t) {
    let mut i: i32 = 0;
    let mut len: i32 = 0;
    let mut src: *mut f32 = grbuf;
    let mut dst: *mut f32 = scratch;
    loop {
        len = *sfb as i32;
        if !(0 as i32 != len) {
            break;
        }
        i = 0 as i32;
        while i < len {
            let fresh19 = dst;
            dst = dst.offset(1);
            *fresh19 = *src.offset((0 as i32 * len) as isize);
            let fresh20 = dst;
            dst = dst.offset(1);
            *fresh20 = *src.offset((1 as i32 * len) as isize);
            let fresh21 = dst;
            dst = dst.offset(1);
            *fresh21 = *src.offset((2 as i32 * len) as isize);
            i += 1;
            src = src.offset(1);
        }
        sfb = sfb.offset(3 as i32 as isize);
        src = src.offset((2 as i32 * len) as isize);
    }
    memcpy(
        grbuf as *mut (),
        scratch as *const (),
        (dst.offset_from(scratch) as isize as usize)
            .wrapping_mul(::core::mem::size_of::<f32>() as usize),
    );
}
unsafe fn L3_antialias(mut grbuf: *mut f32, mut nbands: i32) {
    static g_aa: [[f32; 8]; 2] = [
        [
            0.85749293f32,
            0.88174200f32,
            0.94962865f32,
            0.98331459f32,
            0.99551782f32,
            0.99916056f32,
            0.99989920f32,
            0.99999316f32,
        ],
        [
            0.51449576f32,
            0.47173197f32,
            0.31337745f32,
            0.18191320f32,
            0.09457419f32,
            0.04096558f32,
            0.01419856f32,
            0.00369997f32,
        ],
    ];
    while nbands > 0 as i32 {
        let mut i: i32 = 0 as i32;
        while i < 8 as i32 {
            let mut u: f32 = *grbuf.offset((18 as i32 + i) as isize);
            let mut d: f32 = *grbuf.offset((17 as i32 - i) as isize);
            *grbuf.offset((18 as i32 + i) as isize) =
                u * g_aa[0 as i32 as usize][i as usize] - d * g_aa[1 as i32 as usize][i as usize];
            *grbuf.offset((17 as i32 - i) as isize) =
                u * g_aa[1 as i32 as usize][i as usize] + d * g_aa[0 as i32 as usize][i as usize];
            i += 1;
        }
        nbands -= 1;
        grbuf = grbuf.offset(18 as i32 as isize);
    }
}
unsafe fn L3_dct3_9(mut y: *mut f32) {
    let mut s0: f32 = 0.;
    let mut s1: f32 = 0.;
    let mut s2: f32 = 0.;
    let mut s3: f32 = 0.;
    let mut s4: f32 = 0.;
    let mut s5: f32 = 0.;
    let mut s6: f32 = 0.;
    let mut s7: f32 = 0.;
    let mut s8: f32 = 0.;
    let mut t0: f32 = 0.;
    let mut t2: f32 = 0.;
    let mut t4: f32 = 0.;
    s0 = *y.offset(0 as i32 as isize);
    s2 = *y.offset(2 as i32 as isize);
    s4 = *y.offset(4 as i32 as isize);
    s6 = *y.offset(6 as i32 as isize);
    s8 = *y.offset(8 as i32 as isize);
    t0 = s0 + s6 * 0.5f32;
    s0 -= s6;
    t4 = (s4 + s2) * 0.93969262f32;
    t2 = (s8 + s2) * 0.76604444f32;
    s6 = (s4 - s8) * 0.17364818f32;
    s4 += s8 - s2;
    s2 = s0 - s4 * 0.5f32;
    *y.offset(4 as i32 as isize) = s4 + s0;
    s8 = t0 - t2 + s6;
    s0 = t0 - t4 + t2;
    s4 = t0 + t4 - s6;
    s1 = *y.offset(1 as i32 as isize);
    s3 = *y.offset(3 as i32 as isize);
    s5 = *y.offset(5 as i32 as isize);
    s7 = *y.offset(7 as i32 as isize);
    s3 *= 0.86602540f32;
    t0 = (s5 + s1) * 0.98480775f32;
    t4 = (s5 - s7) * 0.34202014f32;
    t2 = (s1 + s7) * 0.64278761f32;
    s1 = (s1 - s5 - s7) * 0.86602540f32;
    s5 = t0 - s3 - t2;
    s7 = t4 - s3 - t0;
    s3 = t4 + s3 - t2;
    *y.offset(0 as i32 as isize) = s4 - s7;
    *y.offset(1 as i32 as isize) = s2 + s1;
    *y.offset(2 as i32 as isize) = s0 - s3;
    *y.offset(3 as i32 as isize) = s8 + s5;
    *y.offset(5 as i32 as isize) = s8 - s5;
    *y.offset(6 as i32 as isize) = s0 + s3;
    *y.offset(7 as i32 as isize) = s2 - s1;
    *y.offset(8 as i32 as isize) = s4 + s7;
}
unsafe fn L3_imdct36(
    mut grbuf: *mut f32,
    mut overlap: *mut f32,
    mut window: *const f32,
    mut nbands: i32,
) {
    let mut i: i32 = 0;
    let mut j: i32 = 0;
    static g_twid9: [f32; 18] = [
        0.73727734f32,
        0.79335334f32,
        0.84339145f32,
        0.88701083f32,
        0.92387953f32,
        0.95371695f32,
        0.97629601f32,
        0.99144486f32,
        0.99904822f32,
        0.67559021f32,
        0.60876143f32,
        0.53729961f32,
        0.46174861f32,
        0.38268343f32,
        0.30070580f32,
        0.21643961f32,
        0.13052619f32,
        0.04361938f32,
    ];
    j = 0 as i32;
    while j < nbands {
        let mut co: [f32; 9] = [0.; 9];
        let mut si: [f32; 9] = [0.; 9];
        co[0 as i32 as usize] = -*grbuf.offset(0 as i32 as isize);
        si[0 as i32 as usize] = *grbuf.offset(17 as i32 as isize);
        i = 0 as i32;
        while i < 4 as i32 {
            si[(8 as i32 - 2 as i32 * i) as usize] = *grbuf
                .offset((4 as i32 * i + 1 as i32) as isize)
                - *grbuf.offset((4 as i32 * i + 2 as i32) as isize);
            co[(1 as i32 + 2 as i32 * i) as usize] = *grbuf
                .offset((4 as i32 * i + 1 as i32) as isize)
                + *grbuf.offset((4 as i32 * i + 2 as i32) as isize);
            si[(7 as i32 - 2 as i32 * i) as usize] = *grbuf
                .offset((4 as i32 * i + 4 as i32) as isize)
                - *grbuf.offset((4 as i32 * i + 3 as i32) as isize);
            co[(2 as i32 + 2 as i32 * i) as usize] = -(*grbuf
                .offset((4 as i32 * i + 3 as i32) as isize)
                + *grbuf.offset((4 as i32 * i + 4 as i32) as isize));
            i += 1;
        }
        L3_dct3_9(co.as_mut_ptr());
        L3_dct3_9(si.as_mut_ptr());
        si[1 as i32 as usize] = -si[1 as i32 as usize];
        si[3 as i32 as usize] = -si[3 as i32 as usize];
        si[5 as i32 as usize] = -si[5 as i32 as usize];
        si[7 as i32 as usize] = -si[7 as i32 as usize];
        i = 0 as i32;
        while i < 9 as i32 {
            let mut ovl: f32 = *overlap.offset(i as isize);
            let mut sum: f32 = co[i as usize] * g_twid9[(9 as i32 + i) as usize]
                + si[i as usize] * g_twid9[(0 as i32 + i) as usize];
            *overlap.offset(i as isize) = co[i as usize] * g_twid9[(0 as i32 + i) as usize]
                - si[i as usize] * g_twid9[(9 as i32 + i) as usize];
            *grbuf.offset(i as isize) = ovl * *window.offset((0 as i32 + i) as isize)
                - sum * *window.offset((9 as i32 + i) as isize);
            *grbuf.offset((17 as i32 - i) as isize) = ovl * *window.offset((9 as i32 + i) as isize)
                + sum * *window.offset((0 as i32 + i) as isize);
            i += 1;
        }
        j += 1;
        grbuf = grbuf.offset(18 as i32 as isize);
        overlap = overlap.offset(9 as i32 as isize);
    }
}
unsafe fn L3_idct3(mut x0: f32, mut x1: f32, mut x2: f32, mut dst: *mut f32) {
    let mut m1: f32 = x1 * 0.86602540f32;
    let mut a1: f32 = x0 - x2 * 0.5f32;
    *dst.offset(1 as i32 as isize) = x0 + x2;
    *dst.offset(0 as i32 as isize) = a1 + m1;
    *dst.offset(2 as i32 as isize) = a1 - m1;
}
unsafe fn L3_imdct12(mut x: *mut f32, mut dst: *mut f32, mut overlap: *mut f32) {
    static g_twid3: [f32; 6] = [
        0.79335334f32,
        0.92387953f32,
        0.99144486f32,
        0.60876143f32,
        0.38268343f32,
        0.13052619f32,
    ];
    let mut co: [f32; 3] = [0.; 3];
    let mut si: [f32; 3] = [0.; 3];
    let mut i: i32 = 0;
    L3_idct3(
        -*x.offset(0 as i32 as isize),
        *x.offset(6 as i32 as isize) + *x.offset(3 as i32 as isize),
        *x.offset(12 as i32 as isize) + *x.offset(9 as i32 as isize),
        co.as_mut_ptr(),
    );
    L3_idct3(
        *x.offset(15 as i32 as isize),
        *x.offset(12 as i32 as isize) - *x.offset(9 as i32 as isize),
        *x.offset(6 as i32 as isize) - *x.offset(3 as i32 as isize),
        si.as_mut_ptr(),
    );
    si[1 as i32 as usize] = -si[1 as i32 as usize];
    i = 0 as i32;
    while i < 3 as i32 {
        let mut ovl: f32 = *overlap.offset(i as isize);
        let mut sum: f32 = co[i as usize] * g_twid3[(3 as i32 + i) as usize]
            + si[i as usize] * g_twid3[(0 as i32 + i) as usize];
        *overlap.offset(i as isize) = co[i as usize] * g_twid3[(0 as i32 + i) as usize]
            - si[i as usize] * g_twid3[(3 as i32 + i) as usize];
        *dst.offset(i as isize) =
            ovl * g_twid3[(2 as i32 - i) as usize] - sum * g_twid3[(5 as i32 - i) as usize];
        *dst.offset((5 as i32 - i) as isize) =
            ovl * g_twid3[(5 as i32 - i) as usize] + sum * g_twid3[(2 as i32 - i) as usize];
        i += 1;
    }
}
unsafe fn L3_imdct_short(mut grbuf: *mut f32, mut overlap: *mut f32, mut nbands: i32) {
    while nbands > 0 as i32 {
        let mut tmp: [f32; 18] = [0.; 18];
        memcpy(
            tmp.as_mut_ptr() as *mut (),
            grbuf as *const (),
            ::core::mem::size_of::<[f32; 18]>() as usize,
        );
        memcpy(
            grbuf as *mut (),
            overlap as *const (),
            (6 as i32 as usize).wrapping_mul(::core::mem::size_of::<f32>() as usize),
        );
        L3_imdct12(
            tmp.as_mut_ptr(),
            grbuf.offset(6 as i32 as isize),
            overlap.offset(6 as i32 as isize),
        );
        L3_imdct12(
            tmp.as_mut_ptr().offset(1 as i32 as isize),
            grbuf.offset(12 as i32 as isize),
            overlap.offset(6 as i32 as isize),
        );
        L3_imdct12(
            tmp.as_mut_ptr().offset(2 as i32 as isize),
            overlap,
            overlap.offset(6 as i32 as isize),
        );
        nbands -= 1;
        overlap = overlap.offset(9 as i32 as isize);
        grbuf = grbuf.offset(18 as i32 as isize);
    }
}
unsafe fn L3_change_sign(mut grbuf: *mut f32) {
    let mut b: i32 = 0;
    let mut i: i32 = 0;
    b = 0 as i32;
    grbuf = grbuf.offset(18 as i32 as isize);
    while b < 32 as i32 {
        i = 1 as i32;
        while i < 18 as i32 {
            *grbuf.offset(i as isize) = -*grbuf.offset(i as isize);
            i += 2 as i32;
        }
        b += 2 as i32;
        grbuf = grbuf.offset(36 as i32 as isize);
    }
}
unsafe fn L3_imdct_gr(
    mut grbuf: *mut f32,
    mut overlap: *mut f32,
    mut block_type: u32,
    mut n_long_bands: u32,
) {
    static g_mdct_window: [[f32; 18]; 2] = [
        [
            0.99904822f32,
            0.99144486f32,
            0.97629601f32,
            0.95371695f32,
            0.92387953f32,
            0.88701083f32,
            0.84339145f32,
            0.79335334f32,
            0.73727734f32,
            0.04361938f32,
            0.13052619f32,
            0.21643961f32,
            0.30070580f32,
            0.38268343f32,
            0.46174861f32,
            0.53729961f32,
            0.60876143f32,
            0.67559021f32,
        ],
        [
            1 as i32 as f32,
            1 as i32 as f32,
            1 as i32 as f32,
            1 as i32 as f32,
            1 as i32 as f32,
            1 as i32 as f32,
            0.99144486f32,
            0.92387953f32,
            0.79335334f32,
            0 as i32 as f32,
            0 as i32 as f32,
            0 as i32 as f32,
            0 as i32 as f32,
            0 as i32 as f32,
            0 as i32 as f32,
            0.13052619f32,
            0.38268343f32,
            0.60876143f32,
        ],
    ];
    if n_long_bands != 0 {
        L3_imdct36(
            grbuf,
            overlap,
            (g_mdct_window[0 as i32 as usize]).as_ptr(),
            n_long_bands as i32,
        );
        grbuf = grbuf.offset((18 as i32 as u32).wrapping_mul(n_long_bands) as isize);
        overlap = overlap.offset((9 as i32 as u32).wrapping_mul(n_long_bands) as isize);
    }
    if block_type == 2 as i32 as u32 {
        L3_imdct_short(
            grbuf,
            overlap,
            (32 as i32 as u32).wrapping_sub(n_long_bands) as i32,
        );
    } else {
        L3_imdct36(
            grbuf,
            overlap,
            (g_mdct_window[(block_type == 3 as i32 as u32) as i32 as usize]).as_ptr(),
            (32 as i32 as u32).wrapping_sub(n_long_bands) as i32,
        );
    };
}
unsafe fn L3_save_reservoir(mut h: *mut mp3dec_t, mut s: *mut mp3dec_scratch_t) {
    let mut pos: i32 = (((*s).bs.pos + 7 as i32) as u32).wrapping_div(8 as u32) as i32;
    let mut remains: i32 = ((*s).bs.limit as u32)
        .wrapping_div(8 as u32)
        .wrapping_sub(pos as u32) as i32;
    if remains > 511 as i32 {
        pos += remains - 511 as i32;
        remains = 511 as i32;
    }
    if remains > 0 as i32 {
        core::ptr::copy(
            ((*s).maindata).as_mut_ptr().offset(pos as isize),
            ((*h).reserv_buf).as_mut_ptr(),
            remains as usize,
        );
    }
    (*h).reserv = remains;
}
unsafe fn L3_restore_reservoir(
    mut h: *mut mp3dec_t,
    mut bs: *mut bs_t,
    mut s: *mut mp3dec_scratch_t,
    mut main_data_begin: i32,
) -> i32 {
    let mut frame_bytes: i32 = ((*bs).limit - (*bs).pos) / 8 as i32;
    let mut bytes_have: i32 = if (*h).reserv > main_data_begin {
        main_data_begin
    } else {
        (*h).reserv
    };
    memcpy(
        ((*s).maindata).as_mut_ptr() as *mut (),
        ((*h).reserv_buf).as_mut_ptr().offset(
            (if (0 as i32) < (*h).reserv - main_data_begin {
                (*h).reserv - main_data_begin
            } else {
                0 as i32
            }) as isize,
        ) as *const (),
        (if (*h).reserv > main_data_begin {
            main_data_begin
        } else {
            (*h).reserv
        }) as usize,
    );
    memcpy(
        ((*s).maindata).as_mut_ptr().offset(bytes_have as isize) as *mut (),
        ((*bs).buf).offset(((*bs).pos / 8 as i32) as isize) as *const (),
        frame_bytes as usize,
    );
    bs_init(
        &mut (*s).bs,
        ((*s).maindata).as_mut_ptr(),
        bytes_have + frame_bytes,
    );
    return ((*h).reserv >= main_data_begin) as i32;
}
unsafe fn L3_decode(
    mut h: *mut mp3dec_t,
    mut s: *mut mp3dec_scratch_t,
    mut gr_info: *mut L3_gr_info_t,
    mut nch: i32,
) {
    let mut ch: i32 = 0;
    ch = 0 as i32;
    while ch < nch {
        let mut layer3gr_limit: i32 =
            (*s).bs.pos + (*gr_info.offset(ch as isize)).part_23_length as i32;
        L3_decode_scalefactors(
            ((*h).header).as_mut_ptr(),
            ((*s).ist_pos[ch as usize]).as_mut_ptr(),
            &mut (*s).bs,
            gr_info.offset(ch as isize),
            ((*s).scf).as_mut_ptr(),
            ch,
        );
        L3_huffman(
            ((*s).grbuf[ch as usize]).as_mut_ptr(),
            &mut (*s).bs,
            gr_info.offset(ch as isize),
            ((*s).scf).as_mut_ptr(),
            layer3gr_limit,
        );
        ch += 1;
    }
    if (*h).header[3 as i32 as usize] as i32 & 0x10 as i32 != 0 {
        L3_intensity_stereo(
            (*s).grbuf.as_flattened_mut().as_mut_ptr(),
            ((*s).ist_pos[1 as i32 as usize]).as_mut_ptr(),
            gr_info,
            ((*h).header).as_mut_ptr(),
        );
    } else if (*h).header[3 as i32 as usize] as i32 & 0xe0 as i32 == 0x60 as i32 {
        L3_midside_stereo((*s).grbuf.as_flattened_mut().as_mut_ptr(), 576 as i32);
    }
    ch = 0 as i32;
    while ch < nch {
        let mut aa_bands: i32 = 31 as i32;
        let mut n_long_bands: i32 = (if (*gr_info).mixed_block_flag as i32 != 0 {
            2 as i32
        } else {
            0 as i32
        }) << (((*h).header[2 as i32 as usize] as i32 >> 2 as i32
            & 3 as i32)
            + (((*h).header[1 as i32 as usize] as i32 >> 3 as i32 & 1 as i32)
                + ((*h).header[1 as i32 as usize] as i32 >> 4 as i32 & 1 as i32))
                * 3 as i32
            == 2 as i32) as i32;
        if (*gr_info).n_short_sfb != 0 {
            aa_bands = n_long_bands - 1 as i32;
            L3_reorder(
                ((*s).grbuf[ch as usize])
                    .as_mut_ptr()
                    .offset((n_long_bands * 18 as i32) as isize),
                (*s).syn.as_flattened_mut().as_mut_ptr(),
                ((*gr_info).sfbtab).offset((*gr_info).n_long_sfb as i32 as isize),
            );
        }
        L3_antialias(((*s).grbuf[ch as usize]).as_mut_ptr(), aa_bands);
        L3_imdct_gr(
            ((*s).grbuf[ch as usize]).as_mut_ptr(),
            ((*h).mdct_overlap[ch as usize]).as_mut_ptr(),
            (*gr_info).block_type as u32,
            n_long_bands as u32,
        );
        L3_change_sign(((*s).grbuf[ch as usize]).as_mut_ptr());
        ch += 1;
        gr_info = gr_info.offset(1);
    }
}
unsafe fn mp3d_DCT_II(mut grbuf: *mut f32, mut n: i32) {
    static g_sec: [f32; 24] = [
        10.19000816f32,
        0.50060302f32,
        0.50241929f32,
        3.40760851f32,
        0.50547093f32,
        0.52249861f32,
        2.05778098f32,
        0.51544732f32,
        0.56694406f32,
        1.48416460f32,
        0.53104258f32,
        0.64682180f32,
        1.16943991f32,
        0.55310392f32,
        0.78815460f32,
        0.97256821f32,
        0.58293498f32,
        1.06067765f32,
        0.83934963f32,
        0.62250412f32,
        1.72244716f32,
        0.74453628f32,
        0.67480832f32,
        5.10114861f32,
    ];
    let mut i: i32 = 0;
    let mut k: i32 = 0 as i32;
    while k < n {
        let mut t: [[f32; 8]; 4] = [[0.; 8]; 4];
        let mut x: *mut f32 = core::ptr::null_mut();
        let mut y: *mut f32 = grbuf.offset(k as isize);
        x = t.as_flattened_mut().as_mut_ptr();
        i = 0 as i32;
        while i < 8 as i32 {
            let mut x0: f32 = *y.offset((i * 18 as i32) as isize);
            let mut x1: f32 = *y.offset(((15 as i32 - i) * 18 as i32) as isize);
            let mut x2: f32 = *y.offset(((16 as i32 + i) * 18 as i32) as isize);
            let mut x3: f32 = *y.offset(((31 as i32 - i) * 18 as i32) as isize);
            let mut t0: f32 = x0 + x3;
            let mut t1: f32 = x1 + x2;
            let mut t2: f32 = (x1 - x2) * g_sec[(3 as i32 * i + 0 as i32) as usize];
            let mut t3: f32 = (x0 - x3) * g_sec[(3 as i32 * i + 1 as i32) as usize];
            *x.offset(0 as i32 as isize) = t0 + t1;
            *x.offset(8 as i32 as isize) = (t0 - t1) * g_sec[(3 as i32 * i + 2 as i32) as usize];
            *x.offset(16 as i32 as isize) = t3 + t2;
            *x.offset(24 as i32 as isize) = (t3 - t2) * g_sec[(3 as i32 * i + 2 as i32) as usize];
            i += 1;
            x = x.offset(1);
        }
        x = t.as_flattened_mut().as_mut_ptr();
        i = 0 as i32;
        while i < 4 as i32 {
            let mut x0_0: f32 = *x.offset(0 as i32 as isize);
            let mut x1_0: f32 = *x.offset(1 as i32 as isize);
            let mut x2_0: f32 = *x.offset(2 as i32 as isize);
            let mut x3_0: f32 = *x.offset(3 as i32 as isize);
            let mut x4: f32 = *x.offset(4 as i32 as isize);
            let mut x5: f32 = *x.offset(5 as i32 as isize);
            let mut x6: f32 = *x.offset(6 as i32 as isize);
            let mut x7: f32 = *x.offset(7 as i32 as isize);
            let mut xt: f32 = 0.;
            xt = x0_0 - x7;
            x0_0 += x7;
            x7 = x1_0 - x6;
            x1_0 += x6;
            x6 = x2_0 - x5;
            x2_0 += x5;
            x5 = x3_0 - x4;
            x3_0 += x4;
            x4 = x0_0 - x3_0;
            x0_0 += x3_0;
            x3_0 = x1_0 - x2_0;
            x1_0 += x2_0;
            *x.offset(0 as i32 as isize) = x0_0 + x1_0;
            *x.offset(4 as i32 as isize) = (x0_0 - x1_0) * 0.70710677f32;
            x5 = x5 + x6;
            x6 = (x6 + x7) * 0.70710677f32;
            x7 = x7 + xt;
            x3_0 = (x3_0 + x4) * 0.70710677f32;
            x5 -= x7 * 0.198912367f32;
            x7 += x5 * 0.382683432f32;
            x5 -= x7 * 0.198912367f32;
            x0_0 = xt - x6;
            xt += x6;
            *x.offset(1 as i32 as isize) = (xt + x7) * 0.50979561f32;
            *x.offset(2 as i32 as isize) = (x4 + x3_0) * 0.54119611f32;
            *x.offset(3 as i32 as isize) = (x0_0 - x5) * 0.60134488f32;
            *x.offset(5 as i32 as isize) = (x0_0 + x5) * 0.89997619f32;
            *x.offset(6 as i32 as isize) = (x4 - x3_0) * 1.30656302f32;
            *x.offset(7 as i32 as isize) = (xt - x7) * 2.56291556f32;
            i += 1;
            x = x.offset(8 as i32 as isize);
        }
        i = 0 as i32;
        while i < 7 as i32 {
            *y.offset((0 as i32 * 18 as i32) as isize) = t[0 as i32 as usize][i as usize];
            *y.offset((1 as i32 * 18 as i32) as isize) = t[2 as i32 as usize][i as usize]
                + t[3 as i32 as usize][i as usize]
                + t[3 as i32 as usize][(i + 1 as i32) as usize];
            *y.offset((2 as i32 * 18 as i32) as isize) =
                t[1 as i32 as usize][i as usize] + t[1 as i32 as usize][(i + 1 as i32) as usize];
            *y.offset((3 as i32 * 18 as i32) as isize) = t[2 as i32 as usize]
                [(i + 1 as i32) as usize]
                + t[3 as i32 as usize][i as usize]
                + t[3 as i32 as usize][(i + 1 as i32) as usize];
            i += 1;
            y = y.offset((4 as i32 * 18 as i32) as isize);
        }
        *y.offset((0 as i32 * 18 as i32) as isize) = t[0 as i32 as usize][7 as i32 as usize];
        *y.offset((1 as i32 * 18 as i32) as isize) =
            t[2 as i32 as usize][7 as i32 as usize] + t[3 as i32 as usize][7 as i32 as usize];
        *y.offset((2 as i32 * 18 as i32) as isize) = t[1 as i32 as usize][7 as i32 as usize];
        *y.offset((3 as i32 * 18 as i32) as isize) = t[3 as i32 as usize][7 as i32 as usize];
        k += 1;
    }
}

fn mp3d_scale_pcm(sample: f32) -> f32 {
    sample * (1f32 / 32768f32)
}

unsafe fn mp3d_synth_pair(mut pcm: *mut mp3d_sample_t, mut nch: i32, mut z: *const f32) {
    let mut a: f32 = 0.;
    a = (*z.offset((14 as i32 * 64 as i32) as isize) - *z.offset(0 as i32 as isize))
        * 29 as i32 as f32;
    a += (*z.offset((1 as i32 * 64 as i32) as isize) + *z.offset((13 as i32 * 64 as i32) as isize))
        * 213 as i32 as f32;
    a += (*z.offset((12 as i32 * 64 as i32) as isize) - *z.offset((2 as i32 * 64 as i32) as isize))
        * 459 as i32 as f32;
    a += (*z.offset((3 as i32 * 64 as i32) as isize) + *z.offset((11 as i32 * 64 as i32) as isize))
        * 2037 as i32 as f32;
    a += (*z.offset((10 as i32 * 64 as i32) as isize) - *z.offset((4 as i32 * 64 as i32) as isize))
        * 5153 as i32 as f32;
    a += (*z.offset((5 as i32 * 64 as i32) as isize) + *z.offset((9 as i32 * 64 as i32) as isize))
        * 6574 as i32 as f32;
    a += (*z.offset((8 as i32 * 64 as i32) as isize) - *z.offset((6 as i32 * 64 as i32) as isize))
        * 37489 as i32 as f32;
    a += *z.offset((7 as i32 * 64 as i32) as isize) * 75038 as i32 as f32;
    *pcm.offset(0 as i32 as isize) = mp3d_scale_pcm(a);
    z = z.offset(2 as i32 as isize);
    a = *z.offset((14 as i32 * 64 as i32) as isize) * 104 as i32 as f32;
    a += *z.offset((12 as i32 * 64 as i32) as isize) * 1567 as i32 as f32;
    a += *z.offset((10 as i32 * 64 as i32) as isize) * 9727 as i32 as f32;
    a += *z.offset((8 as i32 * 64 as i32) as isize) * 64019 as i32 as f32;
    a += *z.offset((6 as i32 * 64 as i32) as isize) * -(9975 as i32) as f32;
    a += *z.offset((4 as i32 * 64 as i32) as isize) * -(45 as i32) as f32;
    a += *z.offset((2 as i32 * 64 as i32) as isize) * 146 as i32 as f32;
    a += *z.offset((0 as i32 * 64 as i32) as isize) * -(5 as i32) as f32;
    *pcm.offset((16 as i32 * nch) as isize) = mp3d_scale_pcm(a);
}
unsafe fn mp3d_synth(
    mut xl: *mut f32,
    mut dstl: *mut mp3d_sample_t,
    mut nch: i32,
    mut lins: *mut f32,
) {
    let mut i: i32 = 0;
    let mut xr: *mut f32 = xl.offset((576 as i32 * (nch - 1 as i32)) as isize);
    let mut dstr: *mut mp3d_sample_t = dstl.offset((nch - 1 as i32) as isize);
    static g_win: [f32; 240] = [
        -(1 as i32) as f32,
        26 as i32 as f32,
        -(31 as i32) as f32,
        208 as i32 as f32,
        218 as i32 as f32,
        401 as i32 as f32,
        -(519 as i32) as f32,
        2063 as i32 as f32,
        2000 as i32 as f32,
        4788 as i32 as f32,
        -(5517 as i32) as f32,
        7134 as i32 as f32,
        5959 as i32 as f32,
        35640 as i32 as f32,
        -(39336 as i32) as f32,
        74992 as i32 as f32,
        -(1 as i32) as f32,
        24 as i32 as f32,
        -(35 as i32) as f32,
        202 as i32 as f32,
        222 as i32 as f32,
        347 as i32 as f32,
        -(581 as i32) as f32,
        2080 as i32 as f32,
        1952 as i32 as f32,
        4425 as i32 as f32,
        -(5879 as i32) as f32,
        7640 as i32 as f32,
        5288 as i32 as f32,
        33791 as i32 as f32,
        -(41176 as i32) as f32,
        74856 as i32 as f32,
        -(1 as i32) as f32,
        21 as i32 as f32,
        -(38 as i32) as f32,
        196 as i32 as f32,
        225 as i32 as f32,
        294 as i32 as f32,
        -(645 as i32) as f32,
        2087 as i32 as f32,
        1893 as i32 as f32,
        4063 as i32 as f32,
        -(6237 as i32) as f32,
        8092 as i32 as f32,
        4561 as i32 as f32,
        31947 as i32 as f32,
        -(43006 as i32) as f32,
        74630 as i32 as f32,
        -(1 as i32) as f32,
        19 as i32 as f32,
        -(41 as i32) as f32,
        190 as i32 as f32,
        227 as i32 as f32,
        244 as i32 as f32,
        -(711 as i32) as f32,
        2085 as i32 as f32,
        1822 as i32 as f32,
        3705 as i32 as f32,
        -(6589 as i32) as f32,
        8492 as i32 as f32,
        3776 as i32 as f32,
        30112 as i32 as f32,
        -(44821 as i32) as f32,
        74313 as i32 as f32,
        -(1 as i32) as f32,
        17 as i32 as f32,
        -(45 as i32) as f32,
        183 as i32 as f32,
        228 as i32 as f32,
        197 as i32 as f32,
        -(779 as i32) as f32,
        2075 as i32 as f32,
        1739 as i32 as f32,
        3351 as i32 as f32,
        -(6935 as i32) as f32,
        8840 as i32 as f32,
        2935 as i32 as f32,
        28289 as i32 as f32,
        -(46617 as i32) as f32,
        73908 as i32 as f32,
        -(1 as i32) as f32,
        16 as i32 as f32,
        -(49 as i32) as f32,
        176 as i32 as f32,
        228 as i32 as f32,
        153 as i32 as f32,
        -(848 as i32) as f32,
        2057 as i32 as f32,
        1644 as i32 as f32,
        3004 as i32 as f32,
        -(7271 as i32) as f32,
        9139 as i32 as f32,
        2037 as i32 as f32,
        26482 as i32 as f32,
        -(48390 as i32) as f32,
        73415 as i32 as f32,
        -(2 as i32) as f32,
        14 as i32 as f32,
        -(53 as i32) as f32,
        169 as i32 as f32,
        227 as i32 as f32,
        111 as i32 as f32,
        -(919 as i32) as f32,
        2032 as i32 as f32,
        1535 as i32 as f32,
        2663 as i32 as f32,
        -(7597 as i32) as f32,
        9389 as i32 as f32,
        1082 as i32 as f32,
        24694 as i32 as f32,
        -(50137 as i32) as f32,
        72835 as i32 as f32,
        -(2 as i32) as f32,
        13 as i32 as f32,
        -(58 as i32) as f32,
        161 as i32 as f32,
        224 as i32 as f32,
        72 as i32 as f32,
        -(991 as i32) as f32,
        2001 as i32 as f32,
        1414 as i32 as f32,
        2330 as i32 as f32,
        -(7910 as i32) as f32,
        9592 as i32 as f32,
        70 as i32 as f32,
        22929 as i32 as f32,
        -(51853 as i32) as f32,
        72169 as i32 as f32,
        -(2 as i32) as f32,
        11 as i32 as f32,
        -(63 as i32) as f32,
        154 as i32 as f32,
        221 as i32 as f32,
        36 as i32 as f32,
        -(1064 as i32) as f32,
        1962 as i32 as f32,
        1280 as i32 as f32,
        2006 as i32 as f32,
        -(8209 as i32) as f32,
        9750 as i32 as f32,
        -(998 as i32) as f32,
        21189 as i32 as f32,
        -(53534 as i32) as f32,
        71420 as i32 as f32,
        -(2 as i32) as f32,
        10 as i32 as f32,
        -(68 as i32) as f32,
        147 as i32 as f32,
        215 as i32 as f32,
        2 as i32 as f32,
        -(1137 as i32) as f32,
        1919 as i32 as f32,
        1131 as i32 as f32,
        1692 as i32 as f32,
        -(8491 as i32) as f32,
        9863 as i32 as f32,
        -(2122 as i32) as f32,
        19478 as i32 as f32,
        -(55178 as i32) as f32,
        70590 as i32 as f32,
        -(3 as i32) as f32,
        9 as i32 as f32,
        -(73 as i32) as f32,
        139 as i32 as f32,
        208 as i32 as f32,
        -(29 as i32) as f32,
        -(1210 as i32) as f32,
        1870 as i32 as f32,
        970 as i32 as f32,
        1388 as i32 as f32,
        -(8755 as i32) as f32,
        9935 as i32 as f32,
        -(3300 as i32) as f32,
        17799 as i32 as f32,
        -(56778 as i32) as f32,
        69679 as i32 as f32,
        -(3 as i32) as f32,
        8 as i32 as f32,
        -(79 as i32) as f32,
        132 as i32 as f32,
        200 as i32 as f32,
        -(57 as i32) as f32,
        -(1283 as i32) as f32,
        1817 as i32 as f32,
        794 as i32 as f32,
        1095 as i32 as f32,
        -(8998 as i32) as f32,
        9966 as i32 as f32,
        -(4533 as i32) as f32,
        16155 as i32 as f32,
        -(58333 as i32) as f32,
        68692 as i32 as f32,
        -(4 as i32) as f32,
        7 as i32 as f32,
        -(85 as i32) as f32,
        125 as i32 as f32,
        189 as i32 as f32,
        -(83 as i32) as f32,
        -(1356 as i32) as f32,
        1759 as i32 as f32,
        605 as i32 as f32,
        814 as i32 as f32,
        -(9219 as i32) as f32,
        9959 as i32 as f32,
        -(5818 as i32) as f32,
        14548 as i32 as f32,
        -(59838 as i32) as f32,
        67629 as i32 as f32,
        -(4 as i32) as f32,
        7 as i32 as f32,
        -(91 as i32) as f32,
        117 as i32 as f32,
        177 as i32 as f32,
        -(106 as i32) as f32,
        -(1428 as i32) as f32,
        1698 as i32 as f32,
        402 as i32 as f32,
        545 as i32 as f32,
        -(9416 as i32) as f32,
        9916 as i32 as f32,
        -(7154 as i32) as f32,
        12980 as i32 as f32,
        -(61289 as i32) as f32,
        66494 as i32 as f32,
        -(5 as i32) as f32,
        6 as i32 as f32,
        -(97 as i32) as f32,
        111 as i32 as f32,
        163 as i32 as f32,
        -(127 as i32) as f32,
        -(1498 as i32) as f32,
        1634 as i32 as f32,
        185 as i32 as f32,
        288 as i32 as f32,
        -(9585 as i32) as f32,
        9838 as i32 as f32,
        -(8540 as i32) as f32,
        11455 as i32 as f32,
        -(62684 as i32) as f32,
        65290 as i32 as f32,
    ];
    let mut zlin: *mut f32 = lins.offset((15 as i32 * 64 as i32) as isize);
    let mut w: *const f32 = g_win.as_ptr();
    *zlin.offset((4 as i32 * 15 as i32) as isize) = *xl.offset((18 as i32 * 16 as i32) as isize);
    *zlin.offset((4 as i32 * 15 as i32 + 1 as i32) as isize) =
        *xr.offset((18 as i32 * 16 as i32) as isize);
    *zlin.offset((4 as i32 * 15 as i32 + 2 as i32) as isize) = *xl.offset(0 as i32 as isize);
    *zlin.offset((4 as i32 * 15 as i32 + 3 as i32) as isize) = *xr.offset(0 as i32 as isize);
    *zlin.offset((4 as i32 * 31 as i32) as isize) =
        *xl.offset((1 as i32 + 18 as i32 * 16 as i32) as isize);
    *zlin.offset((4 as i32 * 31 as i32 + 1 as i32) as isize) =
        *xr.offset((1 as i32 + 18 as i32 * 16 as i32) as isize);
    *zlin.offset((4 as i32 * 31 as i32 + 2 as i32) as isize) = *xl.offset(1 as i32 as isize);
    *zlin.offset((4 as i32 * 31 as i32 + 3 as i32) as isize) = *xr.offset(1 as i32 as isize);
    mp3d_synth_pair(
        dstr,
        nch,
        lins.offset((4 as i32 * 15 as i32) as isize)
            .offset(1 as i32 as isize),
    );
    mp3d_synth_pair(
        dstr.offset((32 as i32 * nch) as isize),
        nch,
        lins.offset((4 as i32 * 15 as i32) as isize)
            .offset(64 as i32 as isize)
            .offset(1 as i32 as isize),
    );
    mp3d_synth_pair(dstl, nch, lins.offset((4 as i32 * 15 as i32) as isize));
    mp3d_synth_pair(
        dstl.offset((32 as i32 * nch) as isize),
        nch,
        lins.offset((4 as i32 * 15 as i32) as isize)
            .offset(64 as i32 as isize),
    );
    i = 14 as i32;
    while i >= 0 as i32 {
        let mut a: [f32; 4] = [0.; 4];
        let mut b: [f32; 4] = [0.; 4];
        *zlin.offset((4 as i32 * i) as isize) = *xl.offset((18 as i32 * (31 as i32 - i)) as isize);
        *zlin.offset((4 as i32 * i + 1 as i32) as isize) =
            *xr.offset((18 as i32 * (31 as i32 - i)) as isize);
        *zlin.offset((4 as i32 * i + 2 as i32) as isize) =
            *xl.offset((1 as i32 + 18 as i32 * (31 as i32 - i)) as isize);
        *zlin.offset((4 as i32 * i + 3 as i32) as isize) =
            *xr.offset((1 as i32 + 18 as i32 * (31 as i32 - i)) as isize);
        *zlin.offset((4 as i32 * (i + 16 as i32)) as isize) =
            *xl.offset((1 as i32 + 18 as i32 * (1 as i32 + i)) as isize);
        *zlin.offset((4 as i32 * (i + 16 as i32) + 1 as i32) as isize) =
            *xr.offset((1 as i32 + 18 as i32 * (1 as i32 + i)) as isize);
        *zlin.offset((4 as i32 * (i - 16 as i32) + 2 as i32) as isize) =
            *xl.offset((18 as i32 * (1 as i32 + i)) as isize);
        *zlin.offset((4 as i32 * (i - 16 as i32) + 3 as i32) as isize) =
            *xr.offset((18 as i32 * (1 as i32 + i)) as isize);
        let mut j: i32 = 0;
        let fresh22 = w;
        w = w.offset(1);
        let mut w0: f32 = *fresh22;
        let fresh23 = w;
        w = w.offset(1);
        let mut w1: f32 = *fresh23;
        let mut vz: *mut f32 = zlin.offset((4 as i32 * i - 0 as i32 * 64 as i32) as isize);
        let mut vy: *mut f32 =
            zlin.offset((4 as i32 * i - (15 as i32 - 0 as i32) * 64 as i32) as isize);
        j = 0 as i32;
        while j < 4 as i32 {
            b[j as usize] = *vz.offset(j as isize) * w1 + *vy.offset(j as isize) * w0;
            a[j as usize] = *vz.offset(j as isize) * w0 - *vy.offset(j as isize) * w1;
            j += 1;
        }
        let mut j_0: i32 = 0;
        let fresh24 = w;
        w = w.offset(1);
        let mut w0_0: f32 = *fresh24;
        let fresh25 = w;
        w = w.offset(1);
        let mut w1_0: f32 = *fresh25;
        let mut vz_0: *mut f32 = zlin.offset((4 as i32 * i - 1 as i32 * 64 as i32) as isize);
        let mut vy_0: *mut f32 =
            zlin.offset((4 as i32 * i - (15 as i32 - 1 as i32) * 64 as i32) as isize);
        j_0 = 0 as i32;
        while j_0 < 4 as i32 {
            b[j_0 as usize] +=
                *vz_0.offset(j_0 as isize) * w1_0 + *vy_0.offset(j_0 as isize) * w0_0;
            a[j_0 as usize] +=
                *vy_0.offset(j_0 as isize) * w1_0 - *vz_0.offset(j_0 as isize) * w0_0;
            j_0 += 1;
        }
        let mut j_1: i32 = 0;
        let fresh26 = w;
        w = w.offset(1);
        let mut w0_1: f32 = *fresh26;
        let fresh27 = w;
        w = w.offset(1);
        let mut w1_1: f32 = *fresh27;
        let mut vz_1: *mut f32 = zlin.offset((4 as i32 * i - 2 as i32 * 64 as i32) as isize);
        let mut vy_1: *mut f32 =
            zlin.offset((4 as i32 * i - (15 as i32 - 2 as i32) * 64 as i32) as isize);
        j_1 = 0 as i32;
        while j_1 < 4 as i32 {
            b[j_1 as usize] +=
                *vz_1.offset(j_1 as isize) * w1_1 + *vy_1.offset(j_1 as isize) * w0_1;
            a[j_1 as usize] +=
                *vz_1.offset(j_1 as isize) * w0_1 - *vy_1.offset(j_1 as isize) * w1_1;
            j_1 += 1;
        }
        let mut j_2: i32 = 0;
        let fresh28 = w;
        w = w.offset(1);
        let mut w0_2: f32 = *fresh28;
        let fresh29 = w;
        w = w.offset(1);
        let mut w1_2: f32 = *fresh29;
        let mut vz_2: *mut f32 = zlin.offset((4 as i32 * i - 3 as i32 * 64 as i32) as isize);
        let mut vy_2: *mut f32 =
            zlin.offset((4 as i32 * i - (15 as i32 - 3 as i32) * 64 as i32) as isize);
        j_2 = 0 as i32;
        while j_2 < 4 as i32 {
            b[j_2 as usize] +=
                *vz_2.offset(j_2 as isize) * w1_2 + *vy_2.offset(j_2 as isize) * w0_2;
            a[j_2 as usize] +=
                *vy_2.offset(j_2 as isize) * w1_2 - *vz_2.offset(j_2 as isize) * w0_2;
            j_2 += 1;
        }
        let mut j_3: i32 = 0;
        let fresh30 = w;
        w = w.offset(1);
        let mut w0_3: f32 = *fresh30;
        let fresh31 = w;
        w = w.offset(1);
        let mut w1_3: f32 = *fresh31;
        let mut vz_3: *mut f32 = zlin.offset((4 as i32 * i - 4 as i32 * 64 as i32) as isize);
        let mut vy_3: *mut f32 =
            zlin.offset((4 as i32 * i - (15 as i32 - 4 as i32) * 64 as i32) as isize);
        j_3 = 0 as i32;
        while j_3 < 4 as i32 {
            b[j_3 as usize] +=
                *vz_3.offset(j_3 as isize) * w1_3 + *vy_3.offset(j_3 as isize) * w0_3;
            a[j_3 as usize] +=
                *vz_3.offset(j_3 as isize) * w0_3 - *vy_3.offset(j_3 as isize) * w1_3;
            j_3 += 1;
        }
        let mut j_4: i32 = 0;
        let fresh32 = w;
        w = w.offset(1);
        let mut w0_4: f32 = *fresh32;
        let fresh33 = w;
        w = w.offset(1);
        let mut w1_4: f32 = *fresh33;
        let mut vz_4: *mut f32 = zlin.offset((4 as i32 * i - 5 as i32 * 64 as i32) as isize);
        let mut vy_4: *mut f32 =
            zlin.offset((4 as i32 * i - (15 as i32 - 5 as i32) * 64 as i32) as isize);
        j_4 = 0 as i32;
        while j_4 < 4 as i32 {
            b[j_4 as usize] +=
                *vz_4.offset(j_4 as isize) * w1_4 + *vy_4.offset(j_4 as isize) * w0_4;
            a[j_4 as usize] +=
                *vy_4.offset(j_4 as isize) * w1_4 - *vz_4.offset(j_4 as isize) * w0_4;
            j_4 += 1;
        }
        let mut j_5: i32 = 0;
        let fresh34 = w;
        w = w.offset(1);
        let mut w0_5: f32 = *fresh34;
        let fresh35 = w;
        w = w.offset(1);
        let mut w1_5: f32 = *fresh35;
        let mut vz_5: *mut f32 = zlin.offset((4 as i32 * i - 6 as i32 * 64 as i32) as isize);
        let mut vy_5: *mut f32 =
            zlin.offset((4 as i32 * i - (15 as i32 - 6 as i32) * 64 as i32) as isize);
        j_5 = 0 as i32;
        while j_5 < 4 as i32 {
            b[j_5 as usize] +=
                *vz_5.offset(j_5 as isize) * w1_5 + *vy_5.offset(j_5 as isize) * w0_5;
            a[j_5 as usize] +=
                *vz_5.offset(j_5 as isize) * w0_5 - *vy_5.offset(j_5 as isize) * w1_5;
            j_5 += 1;
        }
        let mut j_6: i32 = 0;
        let fresh36 = w;
        w = w.offset(1);
        let mut w0_6: f32 = *fresh36;
        let fresh37 = w;
        w = w.offset(1);
        let mut w1_6: f32 = *fresh37;
        let mut vz_6: *mut f32 = zlin.offset((4 as i32 * i - 7 as i32 * 64 as i32) as isize);
        let mut vy_6: *mut f32 =
            zlin.offset((4 as i32 * i - (15 as i32 - 7 as i32) * 64 as i32) as isize);
        j_6 = 0 as i32;
        while j_6 < 4 as i32 {
            b[j_6 as usize] +=
                *vz_6.offset(j_6 as isize) * w1_6 + *vy_6.offset(j_6 as isize) * w0_6;
            a[j_6 as usize] +=
                *vy_6.offset(j_6 as isize) * w1_6 - *vz_6.offset(j_6 as isize) * w0_6;
            j_6 += 1;
        }
        *dstr.offset(((15 as i32 - i) * nch) as isize) = mp3d_scale_pcm(a[1 as i32 as usize]);
        *dstr.offset(((17 as i32 + i) * nch) as isize) = mp3d_scale_pcm(b[1 as i32 as usize]);
        *dstl.offset(((15 as i32 - i) * nch) as isize) = mp3d_scale_pcm(a[0 as i32 as usize]);
        *dstl.offset(((17 as i32 + i) * nch) as isize) = mp3d_scale_pcm(b[0 as i32 as usize]);
        *dstr.offset(((47 as i32 - i) * nch) as isize) = mp3d_scale_pcm(a[3 as i32 as usize]);
        *dstr.offset(((49 as i32 + i) * nch) as isize) = mp3d_scale_pcm(b[3 as i32 as usize]);
        *dstl.offset(((47 as i32 - i) * nch) as isize) = mp3d_scale_pcm(a[2 as i32 as usize]);
        *dstl.offset(((49 as i32 + i) * nch) as isize) = mp3d_scale_pcm(b[2 as i32 as usize]);
        i -= 1;
    }
}
unsafe fn mp3d_synth_granule(
    mut qmf_state: *mut f32,
    mut grbuf: *mut f32,
    mut nbands: i32,
    mut nch: i32,
    mut pcm: *mut mp3d_sample_t,
    mut lins: *mut f32,
) {
    let mut i: i32 = 0;
    i = 0 as i32;
    while i < nch {
        mp3d_DCT_II(grbuf.offset((576 as i32 * i) as isize), nbands);
        i += 1;
    }
    memcpy(
        lins as *mut (),
        qmf_state as *const (),
        (::core::mem::size_of::<f32>() as usize)
            .wrapping_mul(15 as i32 as usize)
            .wrapping_mul(64 as i32 as usize),
    );
    i = 0 as i32;
    while i < nbands {
        mp3d_synth(
            grbuf.offset(i as isize),
            pcm.offset((32 as i32 * nch * i) as isize),
            nch,
            lins.offset((i * 64 as i32) as isize),
        );
        i += 2 as i32;
    }
    memcpy(
        qmf_state as *mut (),
        lins.offset((nbands * 64 as i32) as isize) as *const (),
        (::core::mem::size_of::<f32>() as usize)
            .wrapping_mul(15 as i32 as usize)
            .wrapping_mul(64 as i32 as usize),
    );
}
unsafe fn mp3d_match_frame(
    mut hdr: *const uint8_t,
    mut mp3_bytes: i32,
    mut frame_bytes: i32,
) -> i32 {
    let mut i: i32 = 0;
    let mut nmatch: i32 = 0;
    i = 0 as i32;
    nmatch = 0 as i32;
    while nmatch < 10 as i32 {
        i += hdr_frame_bytes(hdr.offset(i as isize), frame_bytes)
            + hdr_padding(hdr.offset(i as isize));
        if i + 4 as i32 > mp3_bytes {
            return (nmatch > 0 as i32) as i32;
        }
        if hdr_compare(hdr, hdr.offset(i as isize)) == 0 {
            return 0 as i32;
        }
        nmatch += 1;
    }
    return 1 as i32;
}
unsafe fn mp3d_find_frame(
    mut mp3: *const uint8_t,
    mut mp3_bytes: i32,
    mut free_format_bytes: *mut i32,
    mut ptr_frame_bytes: *mut i32,
) -> i32 {
    let mut i: i32 = 0;
    let mut k: i32 = 0;
    i = 0 as i32;
    while i < mp3_bytes - 4 as i32 {
        if hdr_valid(mp3) != 0 {
            let mut frame_bytes: i32 = hdr_frame_bytes(mp3, *free_format_bytes);
            let mut frame_and_padding: i32 = frame_bytes + hdr_padding(mp3);
            k = 4 as i32;
            while frame_bytes == 0 && k < 2304 as i32 && i + 2 as i32 * k < mp3_bytes - 4 as i32 {
                if hdr_compare(mp3, mp3.offset(k as isize)) != 0 {
                    let mut fb: i32 = k - hdr_padding(mp3);
                    let mut nextfb: i32 = fb + hdr_padding(mp3.offset(k as isize));
                    if !(i + k + nextfb + 4 as i32 > mp3_bytes
                        || hdr_compare(mp3, mp3.offset(k as isize).offset(nextfb as isize)) == 0)
                    {
                        frame_and_padding = k;
                        frame_bytes = fb;
                        *free_format_bytes = fb;
                    }
                }
                k += 1;
            }
            if frame_bytes != 0
                && i + frame_and_padding <= mp3_bytes
                && mp3d_match_frame(mp3, mp3_bytes - i, frame_bytes) != 0
                || i == 0 && frame_and_padding == mp3_bytes
            {
                *ptr_frame_bytes = frame_and_padding;
                return i;
            }
            *free_format_bytes = 0 as i32;
        }
        i += 1;
        mp3 = mp3.offset(1);
    }
    *ptr_frame_bytes = 0 as i32;
    return mp3_bytes;
}

pub const unsafe fn mp3dec_init(mut dec: *mut mp3dec_t) {
    (*dec).header[0 as i32 as usize] = 0 as i32 as u8;
}

pub unsafe fn mp3dec_decode_frame(
    mut dec: *mut mp3dec_t,
    mut mp3: *const uint8_t,
    mut mp3_bytes: i32,
    mut pcm: *mut mp3d_sample_t,
    mut info: *mut mp3dec_frame_info_t,
    scratch: *mut mp3dec_scratch_t,
) -> i32 {
    let mut i: i32 = 0 as i32;
    let mut igr: i32 = 0;
    let mut frame_size: i32 = 0 as i32;
    let mut success: i32 = 1 as i32;
    let mut hdr: *const uint8_t = core::ptr::null();
    let mut bs_frame: [bs_t; 1] = [bs_t {
        buf: core::ptr::null(),
        pos: 0,
        limit: 0,
    }; 1];
    let scratch: &mut mp3dec_scratch_t = &mut *scratch;
    if mp3_bytes > 4 as i32
        && (*dec).header[0 as i32 as usize] as i32 == 0xff as i32
        && hdr_compare(((*dec).header).as_mut_ptr(), mp3) != 0
    {
        frame_size = hdr_frame_bytes(mp3, (*dec).free_format_bytes) + hdr_padding(mp3);
        if frame_size != mp3_bytes
            && (frame_size + 4 as i32 > mp3_bytes
                || hdr_compare(mp3, mp3.offset(frame_size as isize)) == 0)
        {
            frame_size = 0 as i32;
        }
    }
    if frame_size == 0 {
        core::ptr::write_bytes(dec, 0, 1);
        i = mp3d_find_frame(
            mp3,
            mp3_bytes,
            &mut (*dec).free_format_bytes,
            &mut frame_size,
        );
        if frame_size == 0 || i + frame_size > mp3_bytes {
            (*info).frame_bytes = i;
            return 0 as i32;
        }
    }
    hdr = mp3.offset(i as isize);
    memcpy(
        ((*dec).header).as_mut_ptr() as *mut (),
        hdr as *const (),
        4 as i32 as usize,
    );
    (*info).frame_bytes = i + frame_size;
    (*info).frame_offset = i;
    (*info).channels = if *hdr.offset(3 as i32 as isize) as i32 & 0xc0 as i32 == 0xc0 as i32 {
        1 as i32
    } else {
        2 as i32
    };
    (*info).hz = hdr_sample_rate_hz(hdr) as i32;
    (*info).layer = 4 as i32 - (*hdr.offset(1 as i32 as isize) as i32 >> 1 as i32 & 3 as i32);
    (*info).bitrate_kbps = hdr_bitrate_kbps(hdr) as i32;
    if pcm.is_null() {
        return hdr_frame_samples(hdr) as i32;
    }
    bs_init(
        bs_frame.as_mut_ptr(),
        hdr.offset(4 as i32 as isize),
        frame_size - 4 as i32,
    );
    if *hdr.offset(1 as i32 as isize) as i32 & 1 as i32 == 0 {
        get_bits(bs_frame.as_mut_ptr(), 16 as i32);
    }
    if (*info).layer == 3 as i32 {
        let mut main_data_begin: i32 =
            L3_read_side_info(bs_frame.as_mut_ptr(), (scratch.gr_info).as_mut_ptr(), hdr);
        if main_data_begin < 0 as i32
            || (*bs_frame.as_mut_ptr()).pos > (*bs_frame.as_mut_ptr()).limit
        {
            mp3dec_init(dec);
            return 0 as i32;
        }
        success = L3_restore_reservoir(
            dec,
            bs_frame.as_mut_ptr(),
            scratch as *mut mp3dec_scratch_t,
            main_data_begin,
        );
        if success != 0 {
            igr = 0 as i32;
            while igr
                < (if *hdr.offset(1 as i32 as isize) as i32 & 0x8 as i32 != 0 {
                    2 as i32
                } else {
                    1 as i32
                })
            {
                core::ptr::write_bytes(&raw mut scratch.grbuf, 0, 1);
                L3_decode(
                    dec,
                    scratch as *mut mp3dec_scratch_t,
                    (scratch.gr_info)
                        .as_mut_ptr()
                        .offset((igr * (*info).channels) as isize),
                    (*info).channels,
                );
                mp3d_synth_granule(
                    ((*dec).qmf_state).as_mut_ptr(),
                    scratch.grbuf.as_flattened_mut().as_mut_ptr(),
                    18 as i32,
                    (*info).channels,
                    pcm,
                    scratch.syn.as_flattened_mut().as_mut_ptr(),
                );
                igr += 1;
                pcm = pcm.offset((576 as i32 * (*info).channels) as isize);
            }
        }
        L3_save_reservoir(dec, scratch as *mut mp3dec_scratch_t);
    } else {
        return 0 as i32;
    }
    return (success as u32).wrapping_mul(hdr_frame_samples(((*dec).header).as_mut_ptr())) as i32;
}
