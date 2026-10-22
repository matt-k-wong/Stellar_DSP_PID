//! Periodic Error Correction via Wavetable Synthesis
//!
//! ## Physics
//!
//! A worm gear can introduce a position error E(φ) that approximately repeats
//! with worm phase (period T_worm). In right ascension the mount's true angle
//! θ(t) can be modeled as:
//!
//! ```text
//!   θ(t) = Ω·t + E(φ(t))     where φ(t) = 2π·t / T_worm
//! ```
//!
//! The amplitude and repeatability must be measured on the individual mount.
//!
//! ## Feed-Forward Strategy
//!
//! We store samples of −E(φ) (the *inverse* of the measured PE curve)
//! in a length-N lookup table. Playback remains disabled until an external,
//! reliable source synchronizes the oscillator to physical mount phase.
//!
//! At each DSP tick the synthesiser computes the change in desired position
//! since the prior tick and converts that displacement to a guide pulse.
//!
//! The intended effect is to cancel repeatable PE. Improvement depends on
//! curve quality, phase synchronization, pulse response, and mount behavior.
//!
//! ## Interpolation
//!
//! Linear interpolation causes alias artefacts above N/2 Hz.  Hermite
//! cubic (Catmull-Rom) interpolation extends the clean bandwidth to ~N/3 Hz
//! with only 9 multiplies and 9 adds — negligible overhead.
//!
//! ## Calibration
//!
//! Two strategies are supported:
//!
//! 1. **Phase-binning** (`calibrate_from_samples`): bin star-error
//!    measurements into N phase slots over multiple worm cycles, average.
//!    The supplied capture timestamps do not identify physical worm phase, so
//!    a curve produced this way is not ready for playback by itself.
//!
//! 2. **Direct load** (`load_curve`): supply a pre-measured PE vector from
//!    external software (PEMPro, PHD2 PE log).  Resamples to N points with
//!    Hermite interpolation.

/// Catmull-Rom cubic Hermite interpolation over 4 lattice points.
///
/// Gives C¹-continuous output (smooth first derivative).  At fractional
/// position t ∈ [0, 1) between p1 and p2 using context points p0, p3:
///
/// ```text
///   c0 = p1
///   c1 = ½(p2 − p0)
///   c2 = p0 − 2½p1 + 2p2 − ½p3
///   c3 = ½(p3 − p0) + 3/2(p1 − p2)
///   y  = ((c3·t + c2)·t + c1)·t + c0
/// ```
///
/// **9 multiplies · 9 adds · 0 branches · 0 allocations.**
#[inline(always)]
fn hermite4(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let c0 = p1;
    let c1 = 0.5 * (p2 - p0);
    let c2 = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let c3 = 0.5 * (p3 - p0) + 1.5 * (p1 - p2);
    ((c3 * t + c2) * t + c1) * t + c0
}

/// Fixed-size periodic error correction wavetable.
///
/// The const generic parameter `N` is the table length.
/// **Requirements:** N must be a power of two (for O(1) index masking).
/// Typical values: N = 4096 → 32 KB (fits in L1 cache); N = 16384 → 128 KB.
///
/// All allocations occur in `new()`.  The `tick()` method is allocation-free.
pub struct PeriodicErrorTable<const N: usize> {
    /// Inverse PE curve in arcseconds (−E(φ) for feed-forward cancellation)
    table: Box<[f64; N]>,

    // Oscillator state ──────────────────────────────────────────────────────
    /// Current phase accumulator ∈ [0.0, 1.0)
    phase: f64,
    /// Last inverse-PE position sent to the mount, in arcseconds.
    last_correction_arcsec: f64,
    /// Phase advance per DSP sample: 1 / (T_worm · fs)
    phase_increment: f64,
    /// A curve alone is insufficient: playback must be aligned to the mount's
    /// physical worm phase before any correction can be sent.
    curve_loaded: bool,
    phase_synchronized: bool,

    // Physical parameters ───────────────────────────────────────────────────
    /// Worm gear period (seconds)
    pub worm_period: f64,
    /// DSP tick rate (Hz) — used to recompute phase_increment on param change
    sample_rate: f64,
    /// Arcseconds → milliseconds conversion: arcsec / (arcsec_per_ms)
    pub arcsec_per_ms: f64,
    /// Peak-to-peak amplitude of stored curve (arcseconds); 0 if not calibrated
    pub amplitude_pp: f64,
    /// RMS of stored curve (arcseconds)
    pub rms_arcsec: f64,

    // Control flags ─────────────────────────────────────────────────────────
    /// False until a curve is loaded and its phase is explicitly synchronized.
    enabled: bool,
}

impl<const N: usize> PeriodicErrorTable<N> {
    /// Construct with a zeroed table (no PE correction until calibrated).
    ///
    /// # Arguments
    /// * `worm_period`  — Worm gear revolution period (seconds)
    /// * `sample_rate`  — DSP tick rate (Hz)
    /// * `arcsec_per_ms`— Mount RA rate (arcsec per ms of guide pulse)
    pub fn new(worm_period: f64, sample_rate: f64, arcsec_per_ms: f64) -> Self {
        assert!(N.is_power_of_two(), "N must be a power of two");
        assert!(N >= 64, "N must be at least 64");
        Self {
            table:           Box::new([0.0f64; N]),
            phase:           0.0,
            last_correction_arcsec: 0.0,
            phase_increment: if worm_period.is_finite() && worm_period > 0.0 {
                (worm_period * sample_rate).recip()
            } else {
                0.0
            },
            curve_loaded: false,
            phase_synchronized: false,
            worm_period,
            sample_rate,
            arcsec_per_ms,
            amplitude_pp:    0.0,
            rms_arcsec:      0.0,
            enabled:         false,
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // Calibration
    // ─────────────────────────────────────────────────────────────────────

    /// Load a PE curve from an external measurement vector.
    ///
    /// `pe_curve` may have any length; it is resampled to N points using
    /// Hermite interpolation.  The table stores −pe_curve (the inverse).
    ///
    /// # Panics
    /// Panics if `pe_curve` is empty.
    pub fn load_curve(&mut self, pe_curve: &[f64]) {
        assert!(!pe_curve.is_empty(), "pe_curve must not be empty");
        let src_len = pe_curve.len();
        let mut min_val = f64::MAX;
        let mut max_val = f64::MIN;
        let mut sum_sq  = 0.0f64;

        for i in 0..N {
            // Map table index i → fractional source position
            let src_pos = (i as f64 / N as f64) * src_len as f64;
            let idx0    = src_pos as usize;
            let frac    = src_pos.fract();
            // Wrap-safe neighbour indices
            let im1 = if idx0 == 0 { src_len - 1 } else { idx0 - 1 };
            let i1  = (idx0 + 1).min(src_len - 1);
            let i2  = (idx0 + 2).min(src_len - 1);
            // Hermite interpolation → store inverse for feed-forward
            let v = -hermite4(
                pe_curve[im1],
                pe_curve[idx0.min(src_len - 1)],
                pe_curve[i1],
                pe_curve[i2],
                frac,
            );
            self.table[i] = v;
            sum_sq += v * v;
            if v < min_val { min_val = v; }
            if v > max_val { max_val = v; }
        }
        self.amplitude_pp = max_val - min_val;
        self.rms_arcsec   = (sum_sq / N as f64).sqrt();
        self.curve_loaded = true;
        self.last_correction_arcsec = self.position_at_phase(self.phase);
        self.enabled = self.phase_synchronized;
    }

    /// Calibrate from raw star-error time-series measurements.
    ///
    /// `samples` is a slice of `(timestamp_sec, error_arcsec)` pairs.
    ///
    /// **Algorithm:**
    /// 1. Map each timestamp to φ = frac(t / T_worm) using the timestamp
    ///    origin. This is not the mount's physical worm phase unless an
    ///    external phase reference is supplied.
    /// 2. Accumulate measurements into N phase bins (nearest-neighbour).
    /// 3. Forward-fill empty bins by linear interpolation.
    /// 4. Call `load_curve` on the averaged bins.
    ///
    /// A curve built from timestamps alone remains disabled until the caller
    /// separately synchronizes it to a physical worm phase.
    pub fn calibrate_from_samples(&mut self, samples: &[(f64, f64)]) {
        if samples.is_empty() || !self.worm_period.is_finite() || self.worm_period <= 0.0 {
            return;
        }

        let mut sum    = vec![0.0f64; N];
        let mut count  = vec![0u32;   N];

        for &(t, err) in samples {
            let phase = (t / self.worm_period).fract().abs();
            let idx   = (phase * N as f64) as usize & (N - 1);
            sum[idx]   += err;
            count[idx] += 1;
        }

        // Average populated bins
        let mut averaged = vec![0.0f64; N];
        for i in 0..N {
            if count[i] > 0 {
                averaged[i] = sum[i] / count[i] as f64;
            }
        }

        // Linear interpolation for empty bins
        // Two-pass: find populated indices, interpolate gaps
        let populated: Vec<usize> = (0..N).filter(|&i| count[i] > 0).collect();
        if populated.is_empty() { return; }

        let n_pop = populated.len();
        for gap_start in 0..n_pop {
            let i0  = populated[gap_start];
            let i1  = populated[(gap_start + 1) % n_pop];
            let v0  = averaged[i0];
            let v1  = averaged[i1];
            // Number of slots to fill between i0 and i1 (circular)
            let span = if i1 > i0 { i1 - i0 } else { N - i0 + i1 };
            for s in 1..span {
                let t_lerp = s as f64 / span as f64;
                averaged[(i0 + s) % N] = v0 + t_lerp * (v1 - v0);
            }
        }
        self.load_curve(&averaged);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Real-time playback (hot path)
    // ─────────────────────────────────────────────────────────────────────

    /// Advance the phase by one DSP sample and return the change in
    /// feed-forward position as **milliseconds of guide pulse**.
    ///
    /// Positive = West (advances RA); negative = East (retards RA).
    ///
    /// **22 multiplies · 18 adds · 1 branch (enabled check, predicted-taken)
    /// · 0 allocations.**
    #[inline(always)]
    pub fn tick(&mut self) -> f64 {
        if !self.enabled { return 0.0; }

        // Phase advance with branchless wrap
        self.phase += self.phase_increment;
        // Subtract instead of fmod — one branch, one op vs ~20 ops for fmod
        if self.phase >= 1.0 { self.phase -= 1.0; }

        let position_arcsec = self.position_at_phase(self.phase);
        let correction_delta_arcsec = position_arcsec - self.last_correction_arcsec;
        self.last_correction_arcsec = position_arcsec;

        // INDI pulse duration commands a displacement. Convert only the
        // per-tick change in the target position, never the full position
        // curve on every tick.
        correction_delta_arcsec / self.arcsec_per_ms
    }

    #[inline(always)]
    fn position_at_phase(&self, phase: f64) -> f64 {
        let idx_f  = phase * N as f64;
        let idx0   = idx_f as usize & (N - 1);
        let idx_m1 = idx0.wrapping_sub(1) & (N - 1);
        let idx1   = (idx0 + 1) & (N - 1);
        let idx2   = (idx0 + 2) & (N - 1);
        hermite4(
            self.table[idx_m1],
            self.table[idx0],
            self.table[idx1],
            self.table[idx2],
            idx_f.fract(),
        )
    }

    // ─────────────────────────────────────────────────────────────────────
    // Diagnostics / control
    // ─────────────────────────────────────────────────────────────────────

    /// Synchronize the oscillator to a known physical worm phase.
    ///
    /// Calling this method asserts that the supplied phase came from a
    /// reliable mount phase reference. Do not pass zero just to start playback:
    /// a loaded curve without physical phase synchronization remains disabled.
    #[inline]
    pub fn set_phase(&mut self, phase: f64) {
        if !phase.is_finite() {
            self.clear_phase_sync();
            return;
        }
        self.phase = phase.fract().abs();
        self.last_correction_arcsec = self.position_at_phase(self.phase);
        self.phase_synchronized = true;
        self.enabled = self.curve_loaded;
    }

    /// Forget phase synchronization after a reset, power loss, or worm-period
    /// change. A new physical reference is required before playback resumes.
    #[inline]
    pub fn clear_phase_sync(&mut self) {
        self.phase = 0.0;
        self.last_correction_arcsec = 0.0;
        self.phase_synchronized = false;
        self.enabled = false;
    }

    /// Current oscillator phase ∈ [0.0, 1.0).
    #[inline]
    pub fn phase(&self) -> f64 { self.phase }

    /// Whether a loaded curve is armed at a synchronized physical phase.
    #[inline]
    pub fn is_enabled(&self) -> bool { self.enabled }

    /// Update worm period and recompute phase increment.
    pub fn set_worm_period(&mut self, worm_period: f64) {
        self.worm_period      = worm_period;
        self.phase_increment  = if worm_period.is_finite() && worm_period > 0.0 {
            (worm_period * self.sample_rate).recip()
        } else {
            0.0
        };
        self.clear_phase_sync();
    }

    /// Serialise the table to a JSON-compatible vector (for persistence).
    pub fn to_vec(&self) -> Vec<f64> {
        // Negate back to get the original PE curve (not the inverse)
        self.table.iter().map(|&v| -v).collect()
    }

    /// Load from a previously serialised vector (inverse stored automatically).
    pub fn from_vec(&mut self, data: &[f64]) {
        self.load_curve(data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;

    fn make_sinusoidal_pe(n: usize, amplitude: f64) -> Vec<f64> {
        (0..n)
            .map(|i| amplitude * (2.0 * core::f64::consts::PI * i as f64 / n as f64).sin())
            .collect()
    }

    #[test]
    fn tick_returns_zero_when_disabled() {
        let mut tbl: PeriodicErrorTable<4096> =
            PeriodicErrorTable::new(480.0, 10.0, 1.65);
        assert_eq!(tbl.tick(), 0.0);
    }

    #[test]
    fn round_trip_sine_preserves_amplitude() {
        let pe = make_sinusoidal_pe(1024, 5.0);
        let mut tbl: PeriodicErrorTable<4096> =
            PeriodicErrorTable::new(480.0, 10.0, 1.0); // arcsec_per_ms = 1 → pulse_ms = arcsec
        tbl.load_curve(&pe);
        // Peak of stored inverse curve should be ≈ amplitude
        let max_stored: f64 = tbl.table.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        assert_abs_diff_eq!(max_stored, 5.0, epsilon = 0.05);
    }

    #[test]
    fn calibrate_recovers_sinusoid() {
        // Synthetic: generate 3 worm cycles of (t, E(φ)) samples
        let t_worm = 480.0f64;
        let n_cycles = 3u32;
        let fs_guide = 0.5f64; // 0.5 Hz guide rate
        let n_samples = (n_cycles as f64 * t_worm * fs_guide) as usize;
        let amplitude = 8.0f64;
        let samples: Vec<(f64, f64)> = (0..n_samples)
            .map(|i| {
                let t = i as f64 / fs_guide;
                let err = amplitude * (2.0 * core::f64::consts::PI * t / t_worm).sin();
                (t, err)
            })
            .collect();

        let mut tbl: PeriodicErrorTable<1024> =
            PeriodicErrorTable::new(t_worm, 10.0, 1.0);
        tbl.calibrate_from_samples(&samples);
        // RMS of inverse curve ≈ amplitude / √2
        let expected_rms = amplitude / core::f64::consts::SQRT_2;
        assert_abs_diff_eq!(tbl.rms_arcsec, expected_rms, epsilon = 0.3);
    }

    #[test]
    fn hermite_matches_linear_at_lattice_points() {
        // At t=0, hermite4 must equal p1 exactly
        let y = hermite4(0.0, 1.0, 2.0, 3.0, 0.0);
        assert_abs_diff_eq!(y, 1.0, epsilon = 1e-15);
        // At t=1, hermite4 must equal p2
        let y2 = hermite4(0.0, 1.0, 2.0, 3.0, 1.0);
        assert_abs_diff_eq!(y2, 2.0, epsilon = 1e-14);
    }
}
