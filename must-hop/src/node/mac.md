# MAC algorithmer

## Random access

```mermaid
graph TD
    Start([bootup]) --> HasTx{is tx queue empty?}

    HasTx -- No --> Tx{Transmit packets}
    Tx --> Listen

    HasTx -- Yes --> Listen(Listen with timeout)
    Listen --> ListenRes{Got packets <br> before timeout?}

    ListenRes -- Yes --> Conn{Connection okay?}
    Conn -- Yes --> Receive(Receive packets)
    Receive --> Return([Return with packets])

    ListenRes -- No --> ReturnNone([Return None])
    Conn -- No --> ReturnErr([Return conn error])

```

## TDMA

```mermaid
sequenceDiagram
    participant GW as Gateway (Node ID 1)
    participant N1 as Node A (e.g., Hop 1)
    participant N2 as Node B (e.g., Hop 2)

    Note over GW: Frame Start / Slot 0
    GW->>N1: Broadcast Heartbeat (GPS time: T0, Known slots: [0])

    Note over N1: N1 in Rx mode
    N1->>N1: sync_epoch(): Calculate skew vs T0
    N1->>N1: Claims available slot (e.g., Slot 2)
    N1->>N1: Sleeps (Timer::at) until next slot (calc from timestamp)

    Note over GW, N2: Time passes...

    Note over N1: Slot 1 Begins
    N1->>N1: Wakes up for slot
    N1 ->>N1: Calculates timestamp, and current slot
    N1->>N1: Not my slot, listening ...
    N1->>N1: No pkts received, ticking and then sleeping until next slot

    Note over GW, N2: Time passes...

    Note over N1: Slot 2 Begins
    N1->>N1: Wakes up for slot
    N1 ->>N1: Calculates timestamp, and current slot
    N1->>N1: My slot!
    N1->>N1: update_heartbeat(): Writes slots [0, 2] & current timestamp to payload
    N1->>GW: Transmit Heartbeat
    N1->>N2: Transmit Heartbeat

    Note over N2: N2 in Rx mode
    N2->>N2: sync_epoch(): Calculate skew vs N1's timestamp
    N2->>N2: Updates known_slots_mask (sees GW and N1 are taken)
    N2->>N2: Claims available slot (e.g., Slot 3)
    N2->>N2: Sleeps (Timer::at) until next slot

    Note over GW, N2: Time passes...

    Note over N2: Slot 3 Begins
    N2->>N2: Wakes up for its slot
    N2->>N2: update_heartbeat(): Writes slots [2, 3] & current timestamp to payload
    N2->>N1: Transmit Heartbeat
    N2->>GW: Transmit Heartbeat (if in range)
```

### Time synchronization

```mermaid
sequenceDiagram
    participant GW as Gateway
    participant N as Node A

    Note over GW, N: Frame Start / Slot 0
    GW->>N: Broadcast Heartbeat, time: T0

    Note over N: Records T1, takes slot 1
    Note over GW, N: Frame Start / Slot 1
    Note over N: Calc timestamp T2
    N->>GW: Heartbeat (time: T2 )
    Note over GW: Receives T2 at T3 (\Delta_{up})

    Note over GW, N: ... Slot 0
    GW->>N: Broadcast Heartbeat { time: T4, (A: T3) }
    Note over N: T4 - now = \Delta_{down}


```

$$
\text{delay} = \frac{\Delta_{down} + \Delta_{up}}{2}
$$

Delay is how long it takes for a message to get transmitted over the medium and be processed.

$$
\text{offset} = \frac{\Delta_{down} - \Delta_{up}}{2}
$$

Offset is the difference perceived instant at N compared to the GW.

### Calculations with `skew_ratio`

```python
# At timestamp
(old_timestamp, old_instant)
self.timestamp = heartbeat_time + offset
self.sync_instant = now()
gw_diff = self.timestamp - old_timestamp
my_diff = now() - old_instant
self.skew_ratio = gw_diff / my_diff

# At time calculation
elapsed = (now() - self.sync_instant) * self.skew_ratio
time_now = self.timestamp + elapsed

# How much sleep to reach next slot time
(time_now, slot_dur)
elapsed_in_curr_slot = time_now % slot_dur
next_slot_start = slot_dur - elapsed_in_curr_slot
node_offset = next_slot_start / self.skew_ratio
sleep_until = now() + Duration.from_millis(node_offset + guard_band(5ms))

```

```mermaid
graph TD
    Start([bootup]) --> MatchEpoch{Is time sync None?}
    MatchEpoch -- Yes --> listenForHeart(Listen for packets)
    listenForHeart --> IsHeartbeat{Was a packet <br> a heartbeat?}
    IsHeartbeat -- Yes --> callSyncEpoch[call sync_epoch]
    callSyncEpoch --> ret[Return Packets or None]
    IsHeartbeat -- No --> ret


    MatchEpoch -- No --> CalcSlot(Calculate slot)
    CalcSlot --> MatchSlot{Is slot mine?}
    MatchSlot -- Yes --> Tx(Transmit packets)

    Tx --> Sleep(Sleep until next slot)
    Sleep --> ret

    MatchSlot -- No --> IsKnownSlot{Is slot in mask?}
    IsKnownSlot -- Yes --> Rx(listen for packets)
    IsKnownSlot -- No --> ret
    Rx --> ret
```

### `sync_epoch()` Handling a heartbeat packet

```mermaid
graph TD
    IsHeartbeat  --> IsGateway{Am i Gateway or pkt.hops_to_gw > self.hops_to_gw?}
    IsGateway -- Yes --> ClaimSlot[Add Rx slot to own mask]
    ClaimSlot --> IsTxSlotNone{Have i allocated a Tx slot?}
    IsTxSlotNone -- No --> ClaimTxSlot[Claim an available slot from senders mask]
    ClaimTxSlot --> ret([Return ])
    IsTxSlotNone -- Yes --> ret

    %% Is not GW or hops lower
    IsGateway -- No --> IsTimeSyncSet{Is time_sync set?}
    IsTimeSyncSet -- No --> SetTimeSync[Set self.time_sync]
    IsTimeSyncSet -- Yes --> FindSkew[Calculate skew from previous timestamp and instant]
    FindSkew --> SetTimeSync
    SetTimeSync --> ClaimSlot
```
