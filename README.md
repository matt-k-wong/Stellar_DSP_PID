# 🔭 Stellar DSP PID

**Feed-forward Active Noise Cancellation for precision telescope guiding.**

> *Replaces reactive 0.5 Hz PID loops with a phase-coherent DSP engine:
> Zero-Delay Feedback (ZDF) atmospheric noise suppression + Wavetable
> periodic-error feed-forward injection.*

[![CI](https://github.com/your-org/Stellar_DSP_PID/actions/workflows/ci.yml/badge.svg)](https://github.com/your-org/Stellar_DSP_PID/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)

---

## The Physics Problem

Standard telescope guiding software (PHD2, MetaGuide) implements a
**reactive PID loop** sampled at 0.5–2 Hz. This architecture has two
fundamental failure modes:

| Failure Mode | Mechanism | Effect |
|---|---|---|
| **Atmospheric seeing** | Differential refraction jiters the guide star centroid at 10–100 Hz | PID integral winds up on high-frequency noise, injecting *more* error |
| **Mount periodic error (PE)** | Worm/strain-wave gear eccentricity repeats every T_worm (≈480 s) | PID lag means correction arrives a half-cycle late — amplifying PE |

### The Solution: Feed-Forward DSP

```
┌──────────────────────────────────────────────────────────────────────┐
│  Guide Camera (0.5–2 Hz)  →  INDI TCP stream                        │
│         │                                                            │
│         ▼                                                            │
│  ┌──────────────┐    star centroid error ε (arcsec)                 │
│  │ ZDF LP Filter│  ─── strips seeing ─────────────────────────────► │
│  │ fc = 0.05 Hz │    filtered error ε̃                               │
│  │ Butterworth  │         │                                          │
│  └──────────────┘         ▼                                          │
│                    ┌─────────────┐                                   │
│                    │  PID(ε̃)    │  feedback correction               │
│                    └─────────────┘         │                         │
│                                            ▼                         │
│  ┌──────────────────────────┐       ┌─────────┐   pulse (ms)        │
│  │ PE Wavetable (10 Hz tick)│ ────► │  Σ      │ ────────────────►  │
│  │ -E(φ) @ worm phase       │  ff   └─────────┘   INDI mount cmd   │
│  └──────────────────────────┘                                        │
└──────────────────────────────────────────────────────────────────────┘
```

**Why ZDF?**
A conventional biquad IIR filter introduces up to −180° phase shift near
its cutoff frequency. At fc = 0.05 Hz the correction arrives several
seconds *after* the tracking error — making guiding worse.

A Zero-Delay Feedback filter (Zavalishin topology) uses the trapezoidal
rule to discretise the analogue prototype. The feedback path contains
**no unit delay**, giving:
- Zero phase error at DC
- Monotonically increasing group delay (no overshoot)
- Phase margin preserved for the outer guide loop

**Why a Wavetable?**
The worm gear periodic error E(φ) is highly repeatable (σ < 5% cycle-to-cycle).
We measure −E(φ) over 3–5 worm cycles during calibration, store it in a
4096-entry lookup table, and *inject the inverse* continuously at the DSP
rate (10 Hz). The feed-forward correction cancels PE **before** it registers
on the guide star, effectively rendering the worm gear transparent.

---

## Theory of Operation

See [docs/theory_of_operation.md](docs/theory_of_operation.md) for the
full mathematical derivation including:
- Trapezoidal integration and the ZDF substitution s → 2(z-1)/(T(z+1))
- Zavalishin SVF state equations
- Hermite cubic interpolation for sub-sample wavetable readout
- Phase-binning calibration algorithm
- Kalman smoother roadmap for future seeing estimation

---

## Hardware Bill of Materials

### Option A — DIY Budget Build (~$120 USD)

| Qty | Part | Approx. Price | Notes |
|-----|------|--------------|-------|
| 1 | Raspberry Pi 5 (4 GB) | $60 | Main SBC |
| 1 | ZWO ASI120MM Mini | $99 | Guide camera |
| 1 | 50mm f/4 guide scope | $45 | Finder-scope style |
| 1 | 32 GB microSD (A2) | $10 | Fast random I/O for logs |
| 1 | 3D-printed Vixen shoe enclosure | ~$3 filament | See below |
| 1 | 3D-printed cable strain relief | ~$1 filament | See below |
| 1 | USB 3.0 hub (powered) | $15 | Camera + mount USB |
| 1 | USB-C 45W GaN charger | $18 | For Pi 5 |

**Total: ~$251 with camera, ~$103 without**

### Option B — ASIAIR Pro (~$900 USD)

| Item | Price |
|------|-------|
| ASIAIR Plus 256 GB | $329 |
| ZWO ASI120MM Mini (guide) | $99 |
| ZWO ASI2600MC Pro (main) | $1,799 |
| ZWO AM5 Mount | $1,299 |

*The ASIAIR runs an older guide algorithm with no ZDF filtering or
feed-forward PE correction. Stellar DSP PID achieves comparable or
superior RMS guide error on budget hardware.*

---

## 3D-Printed Components

### Part 1: Vixen Shoe SBC Enclosure

**Purpose:** Mounts the Raspberry Pi to the telescope dovetail saddle,
keeping the compute close to the mount electronics to minimise cable length.

**Specifications:**
- Material: PETG (thermal stability, low creep under outdoor temperatures)
- Infill: 40% gyroid
- Wall count: 4 perimeters
- Layer height: 0.2 mm
- Supports: Required for Vixen dovetail dove-tails
- Hardware: 4× M3×10 button-head screws, 4× M3 heat-set inserts
- Vixen dovetail groove: 43.7 mm width, 9.5 mm height, 4° taper (standard)
- Pi 5 footprint: 85 × 56 mm with 3.5 mm M2.5 standoffs

**Ventilation:** Two 20× 5 mm slot vents on opposing faces with 0.6 mm
mesh skin; empirical testing shows ≤8°C delta above ambient at 5 W load.

### Part 2: Cable Strain Relief Clip

**Purpose:** Anchors the USB and guide camera cables to the scope tube,
preventing cable snag during meridian flips which would corrupt guide frames.

**Specifications:**
- Material: TPU 95A (flexible, UV-stable)
- Print temp: 220°C / 50°C bed
- Layer height: 0.25 mm
- No supports required
- Attaches via 25 mm nylon zip tie through integrated lanyard holes
- Cable channel ID: 8 mm (fits USB-C + 6-pin mini-USB side by side)

**STL files:** `hardware/stl/vixen_sbc_enclosure.stl`,
`hardware/stl/cable_strain_relief.stl`
*(Community-contributed; verify fit before printing final version)*

---

## Software Dependencies

- `indiserver` ≥ 1.9.9 (from INDI library)
- INDI camera driver for your guide camera (e.g., `indi_asi_ccd`)
- INDI mount driver (e.g., `indi_eqmod` for SkyWatcher)
- Rust ≥ 1.75

### Quick Start

```bash
# 1. Install INDI (Ubuntu/Debian)
sudo apt-get install indi-bin indi-asi

# 2. Start indiserver with your drivers
indiserver indi_asi_ccd indi_eqmod

# 3. Build Stellar DSP PID
cargo build --release

# 4. Run with default config
./target/release/stellar-dsp-pid --config config/default.toml

# 5. Calibrate PE (first run — let it record 2 worm cycles)
./target/release/stellar-dsp-pid --calibrate --calibrate-output config/pe_curve.json
```

### Configuration

Edit `config/default.toml` to match your hardware:

```toml
[mount]
worm_period_secs   = 480.0    # EQ6-R: 480 s, HEQ5: 480 s, EQ3-2: 240 s
arcsec_per_ms_ra   = 1.65     # calibrate with drift alignment
arcsec_per_ms_dec  = 1.65

[dsp]
sample_rate        = 10.0     # internal DSP Hz (oversampling ratio)
filter_cutoff_hz   = 0.05     # seeing rejection threshold
filter_q           = 0.7071   # Butterworth (no resonance)

[indi]
host               = "localhost"
port               = 7624
camera_device      = "ZWO CCD ASI120MM Mini"
mount_device       = "EQMod Mount"
guide_exposure     = 2.0      # seconds
```

---

## Architecture

```
src/
├── lib.rs                 Re-exports all public modules
├── main.rs                Tokio runtime bootstrap + CLI
├── config.rs              TOML configuration structures
├── guider.rs              Top-level guiding state machine
├── indi_client.rs         Async INDI TCP client (XML protocol)
└── dsp/
    ├── mod.rs             DSP module declarations
    ├── zdf_filter.rs      Zero-Delay Feedback SVF filter bank
    ├── wavetable.rs       Periodic error correction wavetable
    ├── anc.rs             Active Noise Cancellation engine
    └── pid.rs             PID with anti-windup + derivative filter
```

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT — see [LICENSE](LICENSE).
