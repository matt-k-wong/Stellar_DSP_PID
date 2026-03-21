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

A worm-and-wheel drive has eccentricity ε in its mesh.  As the worm rotates,
the RA axis experiences a sinusoidal position modulation:

```
θ_actual(t) = Ω·t + A·sin(2π·t/T_worm + φ_0)
```

The PID controller measures the error caused by this at time t, computes a
correction, and applies it — but the guide pulse executes *after* a full
exposure cycle (2 s) plus communication latency.  With T_worm = 480 s,
a 2–4 s lag represents 1.5–3° of worm phase: the correction arrives when
the PE has the *opposite* sign, amplifying the error instead of cancelling it.

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

The mount's true PE curve E(φ) is measured by recording the RA guide error
over N ≥ 3 complete worm cycles while mount tracking is active and guiding
is disabled (open-loop).  The measurements are phase-binned:

```
φ_i = frac(t_i / T_worm)
bin_index = floor(φ_i · N_table)
bin_sum[k]   += error[i]
bin_count[k] += 1
```

After collection, each bin is averaged.  Empty bins are linearly interpolated.
The RMS of the residual (after subtracting the fitted curve from raw data)
converges as σ_residual ≈ σ_seeing / sqrt(n_cycles · n_hits_per_bin).

### 3.2 Feed-Forward Synthesis

The wavetable stores samples of −E(φ) (the *inverse* PE curve).  A phase
accumulator advances by Δφ = 1/(T_worm · fs) per DSP tick:

```
φ[n+1] = φ[n] + Δφ  (mod 1)
correction[n] = −E(φ[n]) / arcsec_per_ms
```

The readout uses Hermite cubic (Catmull-Rom) interpolation between four
nearest table entries.  This gives C¹ continuity and suppresses interpolation
noise by ~30 dB compared to linear interpolation for the same table size.

### 3.3 Accuracy Budget

For a 4096-point table over T_worm = 480 s, the time resolution per bin is:

```
Δt_bin = T_worm / N = 480/4096 ≈ 0.117 s
```

The maximum interpolation error for Hermite interpolation of a sinusoid is:

```
ε_max ≈ (π·f_PE·Δt_bin)^4 / 384 · A_PE
```

For a 5-arcsecond peak PE: ε_max ≈ 0.0002″ — negligible compared to
centroid measurement noise (σ_centroid ≈ 0.1–0.5″ for typical setups).

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

The ZDF filter decouples the PID from seeing noise; the wavetable removes
the dominant periodic error before it reaches the guide star; the PID
handles residual drift (polar alignment error, refraction, mount flexure).

---

## 5. Oversampling Architecture

The INDI guide frame arrival rate (0.5–2 Hz) is much lower than the
desired correction rate.  Running the DSP engine at a higher internal
rate (`sample_rate`, default 10 Hz) provides:

1. **Smooth PE correction pulses** — the wavetable advances 20× per guide
   frame, so corrections are issued every 100 ms rather than every 2 s.
   This reduces the instantaneous pulse amplitude by 20× for the same
   total correction, reducing mount excitation.

2. **Filter state continuity** — the ZDF filters evolve their states between
   guide frames via a zero-input response.  When a new frame arrives the
   filter already has the correct memory of recent history.

3. **Latency headroom** — at 10 Hz the correction loop can respond within
   100 ms to a sudden mount disturbance even if no new guide frame has arrived.
