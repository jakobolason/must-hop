use std::{env, path::PathBuf};

extern crate cc;

// TODO: Do i have SX1261 for LBT( listen before talk )
fn main() {
    // Build `libtinymt32` (mersenne twister) which `libloragw` depends on.
    cc::Build::new()
        .include("vendor/sx1302_hal/libtools/inc")
        .file("vendor/sx1302_hal/libtools/src/tinymt32.c")
        .compile("tinymt32");

    // Build our extracted, modified, and vendored `libloragw`.
    cc::Build::new()
        .include("vendor/sx1302_hal/libloragw/inc")
        .include("vendor/sx1302_hal/libtools/inc")
        .include("vendor/sx1302_hal_cfg")
        .file("vendor/sx1302_hal/libloragw/src/loragw_aux.c")
        .file("vendor/sx1302_hal/libloragw/src/loragw_cal.c")
        .file("vendor/sx1302_hal/libloragw/src/loragw_debug.c")
        .file("vendor/sx1302_hal/libloragw/src/loragw_hal.c")
        .file("vendor/sx1302_hal/libloragw/src/loragw_i2c.c")
        .file("vendor/sx1302_hal/libloragw/src/loragw_reg.c")
        .file("vendor/sx1302_hal/libloragw/src/loragw_spi.c")
        .file("vendor/sx1302_hal/libloragw/src/loragw_stts751.c")
        .file("vendor/sx1302_hal/libloragw/src/loragw_sx1250.c")
        .file("vendor/sx1302_hal/libloragw/src/loragw_sx125x.c")
        .file("vendor/sx1302_hal/libloragw/src/loragw_sx1302.c")
        .file("vendor/sx1302_hal/libloragw/src/loragw_sx1302_rx.c")
        .file("vendor/sx1302_hal/libloragw/src/loragw_sx1302_timestamp.c")
        .compile("loragw");

    let target = env::var("TARGET").expect("TARGET environment variable not set");

    let bindings = bindgen::Builder::default()
        .header("vendor/bindgen-sx1302.h")
        .clang_arg(format!("--target={target}"))
        .clang_arg("-Ivendor/sx1302_hal_cfg")
        .clang_arg("-Ivendor/sx1302_hal/libloragw/inc")
        .clang_arg("-D__float128=long double")
        .clang_arg("-D__STRICT_ANSI__")
        .clang_arg("-Wno-everything")
        .allowlist_function("lgw_.*")
        .allowlist_type("lgw_.*")
        .allowlist_var("LGW_.*")
        .no_debug("lgw_pkt_tx_s")
        .no_debug("lgw_pkt_rx_s")
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
