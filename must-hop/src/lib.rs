#![no_std]
#![no_main]

use core::module_path;
use defmt::info;
use serde::{Deserialize, Serialize};

pub fn hello_world() {
    info!("Hello world from must hop!");
}

#[derive(Serialize, Deserialize, Debug, PartialEq, defmt::Format)]
pub struct SensorData {
    pub device_id: u8,
    pub temperate: f32,
    pub voltage: f32,
    pub acceleration_x: f32,
}
