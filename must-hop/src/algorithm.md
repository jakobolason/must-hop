# Library Traits and implementations

The `MHNode` trait:

```mermaid
classDiagram
    class MHNode~SIZE, LEN~ {
        <<trait>>
        +type Error
        +type Connection
        +type ReceiveBuffer
        +transmit(packets: &[MHPacket~SIZE~]) Future~Result~
        +receive(conn, rec_buf: Self::ReceiveBuffer) Future~Result~
        +listen(rec_buf: Self::ReceiveBuffer, with_timeout: Option~Duration~) Future~Result~
    }

    class LoraNode~RK, DLY, SIZE, LEN~ {
        <<struct>>
        -&mut LoRa~RK, DLY~ lora
        -TransmitParameters _tp
        -PacketParams pkt_params
        -ModulationParams mdltn_params

        %% Associated Types Defined
        +type Error = RadioError
        +type Connection = Result~Tuple~u8, PacketStatus~, RadioError~
        +type ReceiveBuffer = [u8; 256]
        +type Duration = u16

        +new(lora, tp) Result~LoraNode, RadioError~
        +prepare_for_rx(rx_mode) Result~(), RadioError~

        %% Trait Implementations
        +transmit(packets: &[MHPacket~SIZE~]) Result~(), RadioError~
        +receive(conn, rec_buf) Result~Vec~MHPacket~SIZE~, LEN~, RadioError~
        +listen(rec_buf, with_timeout) Result~Connection, RadioError~
    }

    MHNode <|.. LoraNode : implements
```

- `MHPacket` layout:

```mermaid
---
title: "MHPacket Layout"
---
packet-beta
0-7: "destination_id (8 bits)"
8-15: "packet_type (8 bits)"
16-31: "packet_id (16 bits)"
32-39: "source_id (8 bits)"
40-47: "hop_count (8 bits)"
48-55: "hop_to_gw (8 bits)"
56-95: "payload (max 256 bytes)"
```
