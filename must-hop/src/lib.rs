#![no_std]
#![no_main]

use core::module_path;
use defmt::info;

pub fn hello_world() {
    info!("Hello world from must hop!");
}


