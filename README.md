# Multi hop networking implementation

<div align="center">
  <!-- Crates -->
  <a href="https://crates.io/crates/must-hop">
      <img src="https://img.shields.io/crates/v/must-hop.svg?style=flat-square"
      alt="Crates.io version" />
  </a>
  <!-- Docs -->
  <a href="https://docs.rs/must-hop">
    <img src="https://img.shields.io/badge/docs-latest-blue.svg?style=flat-square"
      alt="docs.rs docs" />
  </a>
  <!-- Downloads -->
  <a href="https://crates.io/crates/must-hop">
    <img src="https://img.shields.io/crates/d/must-hop.svg?style=flat-square"
      alt="Download" />
  </a>
</div>

This project aims to create a multi hop network of multiple sensors in a chain topology sending sensor information to a GW who relays this information to a remote server.

Meant as the source code for my bachelor's thesis, which explores the emerging world of LoRa, embedded Rust and a goal of creating a small device for a sensor network on power lines and reducing the gateway / sensor ratio.

Power consumption should be minimzed such that the footprint of these sensors is minimal, which is why a TDMA implementation is important for the success of this project's goal.

## Contents

- `must-hop`:
  Provides traits for nodes, a NetworkManager to handle the multi hop logic, and a MeshRouter to handle the flow of receiving and retransmitting packages.
  - `MeshRouter` handles a `MHNode` and a `NetworkManager`, then given a policy for replying to messages handles how a node should receive and transmit to create the multi hop network
  - `NetworkManager` Is the brains maintaining a record of sent packets which have not been acknowledged yet. The acknowledgement is handled by the routing policy given to the `MeshRouter`
  - `MacPolicy`: An implementation for Random Access(RA) and TDMA can be chosen for the system, such that it simply a plug-and-play system when compiling, as to how the system should communicate.

- `libloragw-sys`:
  Rust bindings for the sx1302-hal to use the RAK2287 board on a raspberry Pi and communicate to it with a rust program.

- `loragw`:
  Rust wrappers around `libloragw-sys` to be able to interface with it safely, uses a typestate pattern to guide users to a safe API.

- `must-gw`:
  A Lora Gateway to retrieve sensor data from nodes and send them to a remote server. Will use `must-hop` to act as a node on the network, but being special because it replies with ACK's instead of retransmitting packages.

## Examples

The goal is to have 2 working examples, one with the ESP32-C6 dev board which has BLE capabilities, to create a multi hop network. The trouBLE crate provides a nice abstraction on top of the antenna, so implementing the MHNode traits for trouBLE could be enough.

The other example is using the RAK3272s board, holding a RAK3172 inside, which has a STM32WLE5CC and a SemTech 1262 LoRa radio. Here, lora-rs packages are used to provide low-level drivers, and the goal is to create an implementation of the MHNode traits for LoRa without making MHNode to closely coupled to LoRa.

A Justfile has been introduced to ease the use of handling the examples for this project, and to document the different ways of running the examples. To view all available options, run `just -l`.

### Probe-rs server

Probe-rs is a very powerful tool, and by compiling probe-rs with the `server` feature flag, it is possible to run `probe-rs serve` on a remote host, and connect to that server on your local machine. With this, it is possible to launch programs and flash to a device, as if you were directly connected to it. This is utilized in this project, where `just remote-run` runs the RAK3272s example but connects to a Raspberry Pi using Tailscale and lets probe-rs handle the authentication and flashing of my request.
For more information, visit the [docs](https://probe.rs/docs/tools/#serve)

### cross-rs

Using cross-rs, a cross compilation of the `must-gw` binary can be compiled and deployed to a remote host (configured for a Raspberry Pi) given a host url and user as environment variables.

Thus everything can be compiled and then the binaries can be copied to a remote host to be run there instead of using hardwired connections, improving the developer experience.

## Roadmap

- [x] Firmware for RAK3272s
- [x] Nodes can communicate to eachother, with custom messages
- [x] Each node sends and receives information
- [x] Communication with a gateway
  - [x] Define gateway communication
- [x] Dashboard to follow the system at runtime
- [x] Traits required for radios to use this library: `MHNode`
  - [x] lora-rs implements this in `must-hop/lora.rs`
  - [x] SX1302 concentrator implements this in `must-gw/node.rs`
- [ ] medium-access-control somehow handled
  - [x] MacPolicy trait passed to meshrouter, letting the user choose one of the provided implementations or make their own.
  - [x] `RandomAccessMac` defines a naive and simple MAC implementation.
    - [ ] use `lora.cad` for channel activity detection in RA
  - [x] `TDMAMac` defines how MAC is handled if TDMA is chosen.

- [x] Messages can be passed on to another node
  - [x] Define how each packet looks (MHPacket)
  - [x] Algorithm to determine what way to send it
        Will mainly be handled in `NetworkManager`

- [ ] Research power consumption
  - [ ] Look into TDMA to reduce the amount of time the radio is listening, drawing power

### Testing

This project should also be tested, to get some valuable measurements to provide a clear show of what this project has resulted in.

- [ ] Testing functionality
  - [ ] Unit tests
    - [x] Initial ones for NetworkManager
    - [ ] MeshRouter
  - [ ] Integration tests
    - [x] 2 Node simulation communication
    - [x] Simulations of multiple nodes and how packages propagate
    - [ ] Tests with MAC
    - [ ] Hardware-In-Loop (HIL) tests
  - [ ] Amount of errors over time
  - [ ] Durability test, can it run for a week straight?
  - [ ] How many messages per minute can be transmitted?

### Tools to use later

- `cargo-bloab`: Find out what takes most of the space in your executable
- `cargo-call-stack`: Static, Analyze program stack usage
