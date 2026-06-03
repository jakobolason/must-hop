# ONLY TO BE USED ON PI
import math
import signal
import sys
import time
from ctypes import *


SAMPLE_RATE_HZ       = 160_000
TIME_BASE_MS_PER_DIV = 20.0
TRIGGER_POSITION_S   = 0.0

# DIN2 = gateway (trigger); DIN0 and DIN1 = one node each.
# must-dash maps DIN index → node via the ProbeConfig DIN field.
TRIGGER_PIN  = 2
SIGNAL_PINS  = [0, 1]   # measured against the trigger each acquisition

# ── Derived values ─────────────────────────────────────────────────────────
_DIVISIONS            = 10
_WINDOW_S             = (TIME_BASE_MS_PER_DIV / 1000.0) * _DIVISIONS
NUM_SAMPLES           = round(SAMPLE_RATE_HZ * _WINDOW_S)
_POST_TRIGGER_SAMPLES = round(NUM_SAMPLES / 2 - TRIGGER_POSITION_S * SAMPLE_RATE_HZ)

_TRIG_MASK    = 1 << TRIGGER_PIN
_SIGNAL_MASKS = [1 << p for p in SIGNAL_PINS]

# ── SDK setup ──────────────────────────────────────────────────────────────
sys.path.append("/usr/share/digilent/waveforms/samples/py")
try:
    from dwfconstants import *
except ImportError:
    print("Error: dwfconstants.py not found. Is WaveForms installed?")
    sys.exit(1)

dwf = cdll.LoadLibrary("libdwf.so")


def _shutdown(sig, frame):
    raise SystemExit(0)


signal.signal(signal.SIGTERM, _shutdown)
signal.signal(signal.SIGHUP, _shutdown)

hdwf = c_int()
print("Opening Digital Discovery...")
dwf.FDwfDeviceOpen(c_int(-1), byref(hdwf))

if hdwf.value == hdwfNone.value:
    print("Failed to open device. Check USB connection.")
    sys.exit(1)

print("Device opened.")

hzSys = c_double()
dwf.FDwfDigitalInInternalClockInfo(hdwf, byref(hzSys))
_CLOCK_HZ = hzSys.value
_DIVIDER  = int(round(_CLOCK_HZ / SAMPLE_RATE_HZ))

dwf.FDwfDigitalInTriggerAutoTimeoutSet(hdwf, c_double(0))
dwf.FDwfDigitalInDividerSet(hdwf, c_int(_DIVIDER))
dwf.FDwfDigitalInSampleFormatSet(hdwf, c_int(16))
dwf.FDwfDigitalInBufferSizeSet(hdwf, c_int(NUM_SAMPLES))

dwf.FDwfDigitalInTriggerPositionSet(hdwf, c_int(_POST_TRIGGER_SAMPLES))
dwf.FDwfDigitalInTriggerSourceSet(hdwf, trigsrcDetectorDigitalIn)
dwf.FDwfDigitalInTriggerSet(hdwf, c_int(0), c_int(0), c_int(_TRIG_MASK), c_int(0))

print(f"  Sample rate : {SAMPLE_RATE_HZ / 1e6:.4g} MHz  (divider {_DIVIDER})")
print(f"  Window      : {_WINDOW_S * 1000:.1f} ms  ({NUM_SAMPLES} samples)")
print(f"  Trigger     : DIN{TRIGGER_PIN} rising")
print(f"  Signals     : DIN{SIGNAL_PINS[0]} DIN{SIGNAL_PINS[1]}")
print()

rgwData = (c_uint16 * NUM_SAMPLES)()
status  = c_byte()


def find_trigger(data, n):
    """Return sample index of the first rising edge on TRIGGER_PIN, or -1."""
    for i in range(1, n):
        if not (data[i - 1] & _TRIG_MASK) and (data[i] & _TRIG_MASK):
            return i
    return -1


def nearest_rising(data, n, mask):
    """Return sample index of the rising edge on `mask` nearest to sample 0,
    searching the full buffer.  Returns -1 if no rising edge is found."""
    best_idx  = -1
    best_dist = n
    for i in range(1, n):
        if not (data[i - 1] & mask) and (data[i] & mask):
            dist = abs(i)
            if dist < best_dist:
                best_dist = dist
                best_idx  = i
    return best_idx


try:
    while True:
        # Arm
        dwf.FDwfDigitalInConfigure(hdwf, c_int(1), c_int(1))

        # Wait for acquisition to complete
        while True:
            dwf.FDwfDigitalInStatus(hdwf, c_int(1), byref(status))
            if status.value == stsDone.value:
                break
            time.sleep(0.001)

        dwf.FDwfDigitalInStatusData(hdwf, rgwData, c_int(NUM_SAMPLES * 2))

        idx_trig = find_trigger(rgwData, NUM_SAMPLES)

        for pin, mask in zip(SIGNAL_PINS, _SIGNAL_MASKS):
            if idx_trig == -1:
                # No trigger edge found — report sentinel for all signals
                print(f"din{pin}: nan ms", flush=True)
                continue

            # Search for the nearest rising edge on this signal pin.
            # We re-index the buffer relative to the trigger so that the
            # nearest-to-zero edge is the one in the same heartbeat slot.
            best_idx  = -1
            best_dist = NUM_SAMPLES
            for i in range(1, NUM_SAMPLES):
                if not (rgwData[i - 1] & mask) and (rgwData[i] & mask):
                    dist = abs(i - idx_trig)
                    if dist < best_dist:
                        best_dist = dist
                        best_idx  = i

            if best_idx == -1:
                print(f"din{pin}: nan ms", flush=True)
            else:
                delta_ms = (best_idx - idx_trig) / SAMPLE_RATE_HZ * 1000.0
                print(f"din{pin}: {delta_ms:.4f} ms", flush=True)

except (KeyboardInterrupt, SystemExit):
    print("\nStopped.")

# ── Cleanup ───────────────────────────────────────────────────────────────
finally:
    dwf.FDwfDeviceCloseAll()
    print("Device closed.")
