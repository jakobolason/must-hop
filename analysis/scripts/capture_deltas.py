# ONLY TO BE USED ON PI
import sys
import time
from ctypes import *


# SAMPLE_RATE_HZ       = 3_125_000   # Rate field (e.g. 3.125 MHz)
SAMPLE_RATE_HZ       = 160_000   # Rate field (e.g. 3.125 MHz)
TIME_BASE_MS_PER_DIV = 20.0         # Base field (e.g. 1 ms/div)
TRIGGER_POSITION_S   = 0.0         # Position field (0 s = trigger centred in window)

# Which DIN pins to watch. TRIGGER_PIN fires the acquisition; the script then
# finds the nearest rising edge on SIGNAL_PIN and reports the delta.
TRIGGER_PIN = 2   # DIN2 — arms the capture on its rising edge
SIGNAL_PIN  = 1   # DIN1 — the edge we're measuring against

OUTPUT_FILE = "/tmp/timing_delta.csv"

# ── Derived values (no need to touch these) ────────────────────────────────
# WaveForms always shows 10 divisions, so total window = Base × 10.
_DIVISIONS      = 10
_WINDOW_S       = (TIME_BASE_MS_PER_DIV / 1000.0) * _DIVISIONS
# _CLOCK_HZ       = 100_000_000          # Digital Discovery internal clock
# _DIVIDER        = round(_CLOCK_HZ / SAMPLE_RATE_HZ)
NUM_SAMPLES     = round(SAMPLE_RATE_HZ * _WINDOW_S)

# FDwfDigitalInTriggerPositionSet takes the number of samples captured *after*
# the trigger. Position 0 s means the trigger sits at the centre of the buffer.
_POST_TRIGGER_SAMPLES = round(NUM_SAMPLES / 2 - TRIGGER_POSITION_S * SAMPLE_RATE_HZ)

_TRIG_MASK   = 1 << TRIGGER_PIN
_SIGNAL_MASK = 1 << SIGNAL_PIN

# ── SDK setup ──────────────────────────────────────────────────────────────
sys.path.append("/usr/share/digilent/waveforms/samples/py")
try:
    from dwfconstants import *
except ImportError:
    print("Error: dwfconstants.py not found. Is WaveForms installed?")
    sys.exit(1)

dwf = cdll.LoadLibrary("libdwf.so")


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
_DIVIDER = int(round(_CLOCK_HZ / SAMPLE_RATE_HZ))

dwf.FDwfDigitalInTriggerAutoTimeoutSet(hdwf, c_double(0))
dwf.FDwfDigitalInDividerSet(hdwf, c_int(_DIVIDER))
dwf.FDwfDigitalInSampleFormatSet(hdwf, c_int(16))   # 16-bit words, DIN0–DIN15
dwf.FDwfDigitalInBufferSizeSet(hdwf, c_int(NUM_SAMPLES))

# Trigger on the rising edge of TRIGGER_PIN
dwf.FDwfDigitalInTriggerPositionSet(hdwf, c_int(_POST_TRIGGER_SAMPLES))
dwf.FDwfDigitalInTriggerSourceSet(hdwf, trigsrcDetectorDigitalIn)
dwf.FDwfDigitalInTriggerSet(hdwf, c_int(0), c_int(0), c_int(_TRIG_MASK), c_int(0))

print(f"  Sample rate : {SAMPLE_RATE_HZ / 1e6:.4g} MHz  (divider {_DIVIDER})")
print(f"  Window      : {_WINDOW_S * 1000:.1f} ms  ({NUM_SAMPLES} samples)")
print(f"  Trigger     : DIN{TRIGGER_PIN} rising  |  Signal: DIN{SIGNAL_PIN} rising")

print(f"Logging to console -> Ctrl-C to stop\n")

rgwData = (c_uint16 * NUM_SAMPLES)()
status  = c_byte()

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

        # Find the trigger edge (TRIGGER_PIN rising)
        idx_trig = -1
        for i in range(1, NUM_SAMPLES):
            if not (rgwData[i-1] & _TRIG_MASK) and (rgwData[i] & _TRIG_MASK):
                idx_trig = i
                break
        # Find the nearest SIGNAL_PIN rising edge
        idx_sig = -1
        if idx_trig != -1:
            min_dist = NUM_SAMPLES
            for i in range(1, NUM_SAMPLES):
                if not (rgwData[i-1] & _SIGNAL_MASK) and (rgwData[i] & _SIGNAL_MASK):
                    dist = abs(i - idx_trig)
                    if dist < min_dist:
                        min_dist = dist
                        idx_sig = i

        if idx_trig != -1 and idx_sig != -1:
            delta_ms = (idx_sig - idx_trig) / SAMPLE_RATE_HZ * 1000.0
            # with open(OUTPUT_FILE, "a") as f:
            #     f.write(f"{capture_count},{delta_ms:.4f}\n")
            print(f"Capture: {delta_ms:.4f} ms")
        else:
            print(f"Capture: edge not found in buffer.")

except KeyboardInterrupt:
    print("\nStopped.")

# ── Cleanup ───────────────────────────────────────────────────────────────
dwf.FDwfDeviceCloseAll()
print("Device closed.")
