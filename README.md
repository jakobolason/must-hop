# Multi hop networking implementation

This project aims to provide drivers for using a LoRa antenna inspired by the BLE Host controller like trouBLE. And then, on top of such an abstraction layer, create a multi hop network of multiple nodes and connecting to a GW.

Meant as the source code for my bachelor's thesis, which explores the emerging world of LoRa, embedded Rust and a goal of creating a small device for a sensor network on power lines.

## Contents

- `must-hop`:
  Provides traits for nodes, a NetworkManager to handle the multi hop logic, and a MeshRouter to handle the flow of receiving and retransmitting packages.

- `libloragw-sys`:
  Rust bindings for the sx1302-hal to use the RAK2287 board on a raspberry Pi and communicate to it with a rust program.

- `loragw`:
  Rust wrappers around `libloragw-sys` to be able to interface with it safely, uses a typestate pattern to guide users to a safe API.

- `must-gw`:
  Will be: A Lora Gateway to retrieve sensor data from nodes and send them to a remote server. Will use `must-hop` to act as a node on the network, but being special because it replies with ACK's instead of retransmitting packages.
  - [x] Can listen to nodes
  - [ ] Send ACK's back to nodes

## Examples

The goal is to have 2 working examples, one with the ESP32-C6 dev board, which utilizes BLE to create a multi hop network. The trouBLE create provides a nice abstraction on top of the antenna, so implementing the MHNode traits for trouBLE should hopefully be enough.

The other example is using the RAK3272s board, which is a board for the RAK3172 which has a STM32WLE5CC and a SemTech 1262 LoRa radio. Here, lora-rs packages are used to provide low-level drivers, and the goal is to create an implementation of the MHNode traits for LoRa without making MHNode to closely coupled to LoRa, perhaps impossible.

## Roadmap

- [x] Firmware for RAK3272s
- [x] Nodes can communicate to eachother, with custom messages
- [x] Each node sends and receives information
- [x] Communication with a gateway
  - [ ] Define gateway communication
  - [ ] Have a nice dashboard kind of, to see information
- [ ] central, peripheral, runner abstraction for LoRa
  - [ ] Or, use traits to define a transmit and receive function
  - [x] MHNode and NetworkManager define some of the functionlaity required
- [ ] medium-access-control somehow handled
  - [ ] use `lora.cad` for channel activity detection
- [ ] Messages can be passed on to another node
  - [x] Define how each packet looks (MHPacket)
  - [ ] Algorithm to determine what way to send it

### Testing

This project should also be tested, to get some valuable measurements to provide a clear show of what this project has resulted in.

- [ ] Testing functionality
  - [ ] Unit tests
    - [x] Initial ones for NetworkManager
  - [ ] Integration tests
    - [x] Initial tests for 1 node for now
  - [ ] Amount of errors over time
  - [ ] Durability test, can it run for a week straight?
  - [ ] How many messages per minute can be transmitted?
