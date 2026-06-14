# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

All common tasks are in the `Justfile`. Run `just -l` to see all targets. Key ones:

```bash
# Host crates (must-hop, must-gw)
just build          # cargo build -p must-hop && cargo build -p must-gw
just check          # fast check without linking
just clippy         # clippy with -D warnings
just test           # cargo test -p must-hop --features "in_std"
just test-sim       # integration test: network_simulation
just fmt            # cargo fmt --all
just bacon          # live feedback (preferred dev loop)

# Embedded firmware (STM32WLE5CC)
just build-rak      # cargo build --release from examples/lora/rak3272s
just run-rak        # flash via local probe-rs
just remote-run <id>  # flash via remote probe-rs server on Pi (uses .env)

# Gateway (cross-compile for Pi 4B)
just build-gw-pi    # cross build --target aarch64-unknown-linux-gnu
just deploy-gw-pi   # scp binary to Pi
just run-gw         # deploy + run on Pi with sudo chrt -f 90 (real-time priority)

# Dashboard
just dash           # cargo run --release -p must-dash

# Analysis
just analyze-drift <csv>   # run Python analysis on captured timing data
```

The embedded examples (`examples/lora/rak3272s`, `examples/ble/esp32c6`, `examples/gateway/sx1302`) are excluded from the workspace and must be built from their own directories or via `just`.

`.env` (not committed) must be configured from `.env.example` for remote-run/deploy targets — it sets `HOST_URL`, `PI_USER`, and `PROBE_TOKEN`.

## Architecture

This is a multi-hop LoRa mesh network for a bachelor's thesis. A chain of STM32WLE5CC sensor nodes relays data to a Raspberry Pi gateway via TDMA-scheduled LoRa slots.

### Crate roles

**`must-hop`** (`no_std`) — the core library everything depends on:
- `MHNode<SIZE, LEN>` trait: implemented by any radio driver to become a mesh node
- `MeshRouter` wraps an `MHNode` + `NetworkManager` + a `MacPolicy` and drives the network loop via `.tick()`
- `NetworkManager` tracks unacknowledged packets and retransmit logic
- `MacPolicy` trait with two implementations: `RandomAccessMac` (naive) and `TdmaMac` (slot-based)
- All types use `heapless` and `postcard` (no alloc). Feature `in_std` swaps `defmt` for `log` to enable host-side testing.

**`loragw`** (std) — safe typestate wrapper over `libloragw-sys`:
- `Concentrator<State>` with states `Builder → Connected → Running`
- Configuration via TOML (`loragw/cfg/`) parsed into `BoardConf`, `RxRFConf`, `ChannelConf`, `TxGain`
- `ResetToken` enforces that the hardware reset happens before `open()`

**`libloragw-sys`** — raw C bindings to Semtech's sx1302-hal (vendored in `vendor/sx1302_hal/`), built via `cc` + `bindgen` in its `build.rs`

**`must-gw`** (std binary) — gateway that runs on the Pi:
- `create_concentrator()` in `lib.rs` constructs a `Concentrator<Running>` from the TOML config
- `node.rs` wraps the concentrator as a `GWNode` implementing `MHNode`
- Uses `TdmaMac` as the MAC policy; the gateway acts as TDMA leader

**`must-dash`** (std TUI binary) — ratatui dashboard:
- Discovers probe-rs devices, manages PTY processes that run firmware and capture defmt log output
- Two navigation states: Landing (device/config selection) and Dashboard (live monitoring)
- Uses environment variables injected at launch as metadata for experiment runs

**`examples/lora/rak3272s`** — Embassy firmware for STM32WLE5CC + SX1262:
- `build.rs` reads env vars at compile time and generates `env_vars.rs` in `OUT_DIR`, which is `include!`-d in `main.rs`. Currently handles: `SOURCEID` (u8), `KP`/`KI` (i64 PID gains), `ALT_MDLTN` (bool), `SF` (spreading factor 5–12), `BW` (bandwidth in kHz)
- `SF_NUM: u8` and `BW_KHZ: u32` are always-valid numeric consts for defmt logging; `SF: Option<SpreadingFactor>` and `BW: Option<Bandwidth>` are used in the radio setup with `.unwrap_or(default)`
- `lora-phy` is a path dependency pointing to `../../repos/pr-lora-rs/lora-phy` (outside this repo)

### Key cross-crate data flow

```
Sensor firmware (must-hop + lora-phy)
    → LoRa air interface
        → Gateway (must-gw: loragw + must-hop)
            → must-dash (monitors via probe-rs PTY + defmt)
```

### TDMA controller

`must-hop/src/policy/tdma/controller.rs` implements a PTP-style clock sync loop. The gateway is the TDMA leader and broadcasts `SyncBeacon` packets carrying `gps_time_us` and per-node feedback offsets. Nodes use KP/KI gains (configurable via build env vars) to correct their local slot timing.

### `no_std` / `in_std` pattern

`must-hop` compiles for both embedded (no_std) and host (std, for tests). Every logging call is guarded:
```rust
#[cfg(not(feature = "in_std"))]
use defmt::{info, error};
#[cfg(feature = "in_std")]
use log::{info, error};
```
All types that need `defmt::Format` add `#[cfg_attr(not(feature = "in_std"), derive(defmt::Format))]`. Never use `std::` directly in `must-hop`.
