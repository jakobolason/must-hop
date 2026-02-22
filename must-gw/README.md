# Must-GW

A gateway for LoRa written in std Rust. This was made for the RAK2287, and uses `libloragw-sys` bindings from the `sx1302-hal` wrapped safely in `loragw` with the `Concentrator` struct.

The aim is to create a gateway which can listen to the nodes setup for this project, and forward those packets to a remote server.

Perhaps it should also use it's GPS to get a precise time measurement, which could be distributed to the LoRa nodes.

## Install and run

This is supposed to be a binary, meaning it does not expose but has main functions to run. This is std Rust, so simply run

```bash
cargo run
```
