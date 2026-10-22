# Theory of Operation

## 1. The Problem with Reactive PID Guiding

Standard guide software implements a discrete-time PID controller sampled at
the guide camera frame rate *f_guide* ≈ 0.5 Hz:

```
u[n] = Kp·e[n] + Ki·Σe·Δt + Kd·(e[n] − e[n-1])/Δt
```

This loop has two fundamental deficiencies:

### 1.1 Atmospheric Seeing Contamination

Atmospheric turbulence displaces the stellar image at 10–100 Hz.  A 2-second
exposure integrates the seeing, but the centroid measurement still carries
σ_seeing noise.  The PID integral term accumulates this noise and eventually
injects corrections that are *out of phase* with actual tracking errors.

### 1.2 Worm Gear Periodic Error

A worm-and-wheel drive can introduce periodic RA position error. A simplified
model is:

```
θ_actual(t) = Ω·t + A·sin(2π·t/T_worm + φ_0)
```

Feedback latency can matter, but the worm period, error shape, and cycle phase
must be measured for the actual mount. There is no universal 480 s period for
the supported mounts. The illustrative SkyView profile records 598.36 s as a
144-tooth calculation and notes the conflicting 480 s Orion PEC-training
interval; neither value is a measurement of that particular mount.

---

## 2. Zero-Delay Feedback Filters

### 2.1 The Unit-Delay Problem

A conventional first-order IIR low-pass filter is:

```
H(z) = (1−α) / (1 − α·z^{−1})     α = exp(−2π·fc/fs)
```

The `z^{-1}` term is a unit delay in the feedback path.  Its phase response is:

```
∠H(e^{jω}) = −arctan(sin ω / (cos ω − α))
```

At ω→0 (DC) the phase → 0.  But at ω = 2π·fc/fs the phase is −45° for a
1-pole filter.  With fc = 0.05 Hz and fs = 10 Hz, ω_c = 0.031 rad/sample,
and the group delay τ_g = −d∠H/dω ≈ 3.2 samples = 0.32 s at the cutoff.

For fc = 0.05 Hz and the slow drift rates we care about, even this modest
delay shifts the correction by ~20% of the worm phase — enough to reduce
correction efficiency by sin(20°) ≈ 34%.

### 2.2 The Trapezoidal Integrator

The ZDF approach replaces the unit-delay integrator

```
H_int(z) = T/(z−1)         (Euler/forward-difference)
```

with the trapezoidal (bilinear) integrator:

```
H_trap(z) = T(z+1)/(2(z−1))
```

This is equivalent to the substitution s → 2(z-1)/(T(z+1)) — the bilinear
transform from analogue-prototype to digital.

### 2.3 The Zavalishin SVF

The Zavalishin State Variable Filter (SVF) topology places two trapezoidal
integrators in series with a resonance feedback:

```
     x ─── (+) ──┬── [1/s] ── v1 ─── [1/s] ── v2 ─── LP
                 │                     │
                 └──── k ──────────────┘
                               │
                               └──────────────────── BP
     HP = x − k·v1 − v2
```

Discretising both integrators with the trapezoidal rule and solving the
resulting linear system simultaneously (no sequential approximation) gives
the ZDF equations that have **zero excess phase** at DC.

The per-sample computation is:

```
g   = tan(π·fc/fs)
k   = 1/Q
a1  = 1/(1 + g·(g+k))
a2  = g·a1
a3  = g·a2

v3  = x − ic2eq
v1  = a1·ic1eq + a2·v3
v2  = ic2eq + a2·ic1eq + a3·v3
ic1eq' = 2·v1 − ic1eq
ic2eq' = 2·v2 − ic2eq
```

Total cost: 9 multiplies, 8 adds, 0 divisions, 0 branches per sample.

---

## 3. Wavetable PE Cancellation

### 3.1 Measurement

The mount's true PE curve E(φ) must be measured by recording RA guide error
while tracking is active and mount guide output is disabled. A physical worm
phase reference is required to map timestamps to repeatable phase bins. The
current capture command bins by timestamps but has no physical phase input, so
its output is an analysis artifact and is not replay-ready.

```
φ_i = measured physical worm phase at sample i
bin_index = floor(φ_i · N_table)
bin_sum[k]   += error[i]
bin_count[k] += 1
```

After collection, populated bins can be averaged and empty bins interpolated.
Fit quality must be evaluated against raw data across multiple cycles; a
convergence formula does not replace measurements of noise, drift, or
cycle-to-cycle repeatability.

### 3.2 Feed-Forward Synthesis

The wavetable stores samples of −E(φ) (the inverse PE position curve). Playback
is disabled until the oscillator is synchronized to a known physical worm
phase. A phase accumulator then advances by Δφ = 1/(T_worm · fs) per DSP tick:

```
φ[n+1] = φ[n] + Δφ  (mod 1)
pulse[n] = (−E(φ[n+1]) + E(φ[n])) / arcsec_per_ms
```

The pulse uses only the change in desired position at this tick; applying the
full position as a pulse on every tick would repeatedly command the same
displacement. The readout uses Hermite cubic (Catmull-Rom) interpolation
between four nearest table entries. The phase must be synchronized again after
period changes or loss of mount phase.

### 3.3 Accuracy Budget

For a 4096-point table and a known period T_worm, the nominal time resolution
per bin is:

```
Δt_bin = T_worm / N
```

The maximum interpolation error for Hermite interpolation of a sinusoid is:

```
ε_max ≈ (π·f_PE·Δt_bin)^4 / 384 · A_PE
```

Interpolation error depends on measured curve shape and sampling. It should be
compared with the actual centroid noise and mount repeatability, rather than a
generic performance claim.

---

## 4. The Hybrid ANC Loop

The complete correction is:

```
u[n] = ff_PE[n] + a · PID(ε̃[n])
```

where:
- `ff_PE[n]` = wavetable feed-forward (ms)
- `ε̃[n]` = ZDF-LP-filtered guide star error (arcseconds)
- `a` = aggressiveness scalar ∈ [0,1]
- `PID(·)` = discrete-time PID with trapezoidal integration

The ZDF filter can shape feedback bandwidth; a correctly measured and
phase-synchronized wavetable may compensate repeatable periodic error; PID
feedback handles residual guide error. The supplied profiles keep PE disabled
until measurement and phase synchronization are available.

---

## 5. Oversampling Architecture

The INDI guide frame arrival rate (0.5–2 Hz) is much lower than the
desired correction rate.  Running the DSP engine at a higher internal
rate (`sample_rate`, default 10 Hz) provides:

1. **More frequent DSP updates** — the wavetable can advance between guide
   frames. Actual mount pulse size, driver timing, minimum pulse width, and
   accumulated correction need to be validated for the selected mount.

2. **Filter state continuity** — the ZDF filters evolve their states between
   guide frames via a zero-input response.  When a new frame arrives the
   filter already has the correct memory of recent history.

3. **No new visual information** — a higher DSP tick rate does not provide a
   new camera measurement between guide frames. It must not be treated as
   evidence that the mount error is known at each internal tick.
