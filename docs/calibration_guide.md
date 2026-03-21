# Calibration Guide

## Step 1: Baseline Polar Alignment

Before calibrating PE, ensure your polar alignment error is < 5 arcminutes.
Poor polar alignment introduces a secular RA/DEC drift that contaminates the
PE curve measurement.

Use your preferred drift alignment tool (SharpCap, PoleMaster, PHD2 drift).

## Step 2: Plate Scale Measurement (`arcsec_per_ms_ra`)

The feed-forward correction must be scaled correctly to the mount's guide
rate.  Measure `arcsec_per_ms_ra` as follows:

1. Disable guiding and tracking corrections.
2. Send a 1000 ms West pulse via INDI:
   ```
   newNumberVector device="EQMod Mount" name="TELESCOPE_TIMED_GUIDE_WE"
     oneNumber name="TIMED_GUIDE_W" value=1000
   ```
3. Measure the star displacement in arcseconds from the guide frame before
   and after the pulse.
4. `arcsec_per_ms_ra = displacement_arcsec / 1000.0`

Typical value for EQ6-R at 0.5× guide rate: 1.65 arcsec/ms.

## Step 3: Record PE Curve

With the `arcsec_per_ms_ra` value known, run:

```bash
./target/release/stellar-dsp-pid calibrate \
  --cycles 3 \
  --output config/pe_curve.json
```

The calibration mode:
- Connects to indiserver
- Records raw guide star RA deviation for `3 × T_worm` seconds
- Phase-bins and averages the measurements
- Writes the PE curve to `config/pe_curve.json`

**Minimum recommended recording time:** 2 complete worm cycles
(16 minutes for EQ6-R / HEQ5).

## Step 4: Inspect the PE Curve

The PE curve JSON is a simple array of arcseconds values.  You can inspect
it with any plotting tool:

```python
import json, matplotlib.pyplot as plt
with open("config/pe_curve.json") as f:
    pe = json.load(f)
plt.plot(pe)
plt.xlabel("Phase bin (0 = start of worm cycle)")
plt.ylabel("PE (arcseconds)")
plt.title("Periodic Error Curve")
plt.show()
```

A good PE curve is smooth and sinusoidal.  Spike artefacts indicate
seeing or clouds during recording.  If the curve looks noisy, re-run
with `--cycles 5` or on a better night.

## Step 5: Enable PE Correction in Config

```toml
[guider]
enable_pe_correction = true
pe_curve_path = "config/pe_curve.json"
```

## Step 6: Verify with a Test Run

Run the guider for one worm cycle and compare:
- `stats.ff_rms_ms` — feed-forward magnitude
- `stats.ra_error_rms` — guide star RA rms

With effective PE correction `stats.ra_error_rms` should drop by 30–70%
compared to a baseline run with PE correction disabled.
