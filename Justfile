set shell := ["bash", "-c"]
set dotenv-load := true
PI_HOST := env_var_or_default("HOST_URL", "localhost")
PI_USER := env_var_or_default("PI_USER", "pi")
PI_TARGET_DIR := "/home/" + PI_USER + "/must-hop"

# Build must-hop and must-gw
[group('Host builds')]
build:
    echo "Building host crates..."
    cargo build -p must-hop
    cargo build -p must-gw

# Run clippy on the host packages
[group('Host builds')]
clippy:
    cargo clippy -p must-hop -- -D warnings
    cargo clippy -p must-gw -- -D warnings

# Check all host code without building binaries (fast)
[group('Host builds')]
check:
    cargo check -p must-hop
    cargo check -p must-gw

# Clean the target directory
[group('Host builds')]
clean:
    cargo clean

# Run the tests for must-hop
[group('Tests')]
test:
    cargo test -p must-hop --features "in_std"

# runs the network_simulation test for must hop
[group('Tests')]
test-sim:
    cargo test --test network_simulation --features "in_std"

# Use these to hint at using bacon, because this just takes more to type
# Start Bacon for live feedback
[group('Tests')]
bacon:
    bacon

# Start Bacon for testing
[group('Tests')]
bacon-test:
    bacon test

# Note: Requires riscv32imac-unknown-none-elf target installed
[group('examples')]
build-ble:
    @echo "Building BLE ESP32C6 example..."
    cd examples/ble/esp32c6 && cargo build --release

# Flash the ESP32C6
[group('examples')]
flash-ble:
    cd examples/ble/esp32c6 && cargo espflash flash --monitor

# Build the RAK3272s LoRa example
[group('examples')]
build-rak:
    @echo "Building RAK3272s LoRa example..."
    cd examples/lora/rak3272s && cargo build --release

# Flash the RAK3272s example to the chip using probe-rs
[group('examples')]
run-rak:
    @echo "Running RAK3272s LoRa example ..."
    cd examples/lora/rak3272s && cargo run --release --bin main

# --- Runs remotely on Pi

# Runs the RAK3272s example ona  remote probe-rs server
[group('probe-rs')]
remote-run:
    @echo "Flashing remotely to Pi..."
    cd examples/lora/rak3272s && \
    CARGO_TARGET_THUMBV7EM_NONE_EABI_RUNNER="probe-rs run --chip STM32WLE5CC \
    --speed 1000 --connect-under-reset --host ws://"$HOST_URL":3000 --token=$PROBE_TOKEN" \
    cargo run --release --bin main

# Attach to a remote probe-rs server
[group('probe-rs')]
remote-attach:
    @echo "Attaching to remote"
    cd examples/lora/rak3272s && probe-rs attach \
        --chip STM32WLE5CC \
        --host ws://"$HOST_URL":3000 \
        --token="$PROBE_TOKEN" \
        target/thumbv7em-none-eabi/release/main

# Build the SX1302 Gateway example (Host)
[group('examples')]
build-gw-ex:
    @echo "Building SX1302 Gateway example..."
    cd examples/gateway/sx1302 && cargo build

# --- Deployments ---

[group('Pi deployments')]
build-gw-pi:
  @echo "Cross compiling must-gw for Pi 4B"
  cross build --target aarch64-unknown-linux-gnu -p must-gw --release

[group('Pi deployments')]
deploy-gw-pi: build-gw-pi
  @echo "Copying binary GW to pi"
  ssh {{PI_USER}}@{{PI_HOST}} 'mkdir -p {{PI_TARGET_DIR}}/target/aarch64-unknown-linux-gnu/release'
  scp target/aarch64-unknown-linux-gnu/release/must-gw {{PI_USER}}@{{PI_HOST}}:{{PI_TARGET_DIR}}/target/aarch64-unknown-linux-gnu/release/must-gw

[group('Pi deployments')]
run-gw: deploy-gw-pi
  @echo "Running GW on pi"
  ssh {{PI_USER}}@{{PI_HOST}} 'chmod +x {{PI_TARGET_DIR}}/target/aarch64-unknown-linux-gnu/release/must-gw \
    && {{PI_TARGET_DIR}}/target/aarch64-unknown-linux-gnu/release/must-gw'

# Format all code in the workspace
[group('utils')]
fmt:
    cargo fmt --all

# Update dependencies
[group('utils')]
update:
    cargo update
