#![no_std]
#![no_main]

use defmt::info;

pub mod lora;
pub mod node;

pub fn hello_world() {
    info!("Hello world from must hop!");
}
