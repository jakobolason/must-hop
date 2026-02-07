# Multi hop networking implementation

This project aims to provide drivers for using a LoRa antenna just like a BLE Host controller like trouBLE. And then, on top of such an abstraction layer, create a multi hop network of multiple nodes and connecting to a GW.

Meant as the source code for my bachelor's thesis, which explores the emerging world of LoRa, embedded Rust and a goal of creating a small device for a sensor network on power lines.

## Examples

The goal is to have 2 working examples, one with the ESP32-C6 dev board, which utilizes BLE to create a multi hop network. The trouBLE create provides a nice abstraction on top of the antenna, so hopefully no drivers are needed here.

The other example is using the RAK3272s board, which is a board for the RAK3172 which has a STM32WLE5CC and a SemTech 1262 LoRa radio. Here, lora-rs packages are used to provide low-level drivers, and the goal is to create a central, peripheral and runner trio like trouBLE provides, and require that as a trait for others to also use this library.
