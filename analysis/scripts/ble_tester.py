import socket
import struct
import time
import argparse

# --- CONFIGURATION ---
# Use the MAC address from your ESP32 logs.
# Your logs showed: "5A:E6:C5:DF:4B:CD" (or the random FF:E4:05:1A:8F:FF)
# MAC_ADDRESS = "5A:E6:C5:DF:4B:CD"

# The L2CAP Protocol/Service Multiplexer you defined in Rust
PSM = 0x0081

# Address types: 1 = Public (Standard), 2 = Random (Because you used Address::random)
ADDR_TYPE = 1


def main():
    parser = argparse.ArgumentParser(
        prog="BLE L2CAP tester",
        description="Allows connecting to tester",
        epilog="Takes in the MAC of the wanted device",
    )
    parser.add_argument("-m", "--mac")
    args = parser.parse_args()
    mac_address = args.mac
    # SOCK_SEQPACKET is required for Bluetooth L2CAP Connection-Oriented Channels
    sock = socket.socket(
        socket.AF_BLUETOOTH, socket.SOCK_SEQPACKET, socket.BTPROTO_L2CAP
    )

    print(f"Attempting to connect to {mac_address} on PSM {PSM}...")

    try:
        # In Python's AF_BLUETOOTH for LE L2CAP, the tuple is: (MAC, PSM, Address Type)
        # sock.connect((mac_address, PSM, ADDR_TYPE))
        address_type = socket.BDADDR_LE_RANDOM
        sock.connect((mac_address, PSM, 0, address_type))
        print("Successfully connected L2CAP channel!\n")

        while True:
            # We expect 27 bytes based on your Rust `const PAYLOAD_LEN: usize = 27;`
            data = sock.recv(27)
            if not data:
                print("Connection closed by ESP32.")
                break

            print(f"Raw hex received: {data.hex()}")

            # Postcard serialization is extremely lean.
            # A struct of { i8, i8 } is literally just 2 sequential bytes.
            # struct.unpack('<bb') reads 2 signed bytes (little-endian)
            if len(data) >= 2:
                temperature, voltage = struct.unpack("<bb", data[:2])
                print(
                    f"Decoded -> Temperature: {temperature}°C | Voltage: {voltage}V\n"
                )

    except PermissionError:
        print("Error: L2CAP sockets often require root. Run script with 'sudo'.")
    except Exception as e:
        print(f"Connection failed: {e}")
    finally:
        sock.close()


if __name__ == "__main__":
    main()
