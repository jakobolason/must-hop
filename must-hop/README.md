# Must Hop

A multi hop network implementation in rust (rust multi hop).

Abstracts away from the hardware used, and just requires a send and receive function to be a must-hop node. To be used together with a must-gw such that data can be sent to a remote server.

## Examples

Implementations for `lora-phy`s `LoRa` is implemented in `lora.rs` and `tasks/lora.rs`, which provide an implementation for the `MHNode` traits, whilst not assuming the underlying chip and radio (has to be `lora-phy` compatible).

Also includes `SensorData` a simple testing packet which can be used, but the payload of `MHPacket` is user defined, and can be anything reasonably sized and made into byte slices.
