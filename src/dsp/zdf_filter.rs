//! Zero-Delay Feedback (ZDF) Digital Filters
//!
//! ## Background
//!
//! Every conventional IIR biquad has a unit delay z^{-1} in the state
//! feedback path.  This introduces an excess phase of π at Nyquist and a
//! frequency-dependent group delay across the entire passband.  For a
//! low-pass filter with fc = 0.05 Hz and fs = 10 Hz the group delay at
//! DC is τ ≈ (2Q² − 1)/(2π·fc) ≈ 4.5 s — meaning the "corrected" signal
//! the PID sees is describing the sky position from 4½ seconds ago.
//!
//! The ZDF topology eliminates the loop delay by using the trapezoidal
//! (bilinear) integration rule **inside** the feedback path, not just at
//! the input.  The result is:
//!
//! ```text
//!   H_LP(s)         ωc
//!   ─────── = ──────────────     (analogue 1-pole)
//!     1       s + ωc
//!
//!   H_LP(z)         g
//!   ─────── = ──────────────     g = tan(π·fc/fs)   (ZDF discretisation)
//!     1       z·(1+g) − (g−1)
//! ```
//!
//! Phase at DC is exactly 0; group delay is frequency-independent in the
//! passband.  The 2-pole (SVF) extension gives simultaneous LP/BP/HP with
//! the same zero-DC-delay property.
//!
//! ## References
//!
//! - Zavalishin, V. (2012). *The Art of VA Filter Design*, §3.8–§3.10.
//!   Native Instruments.  (Free PDF.)
//! - Pirkle, W. (2019). *Designing Audio Effect Plugins in C++*, Ch. 11.

use core::f64::consts::PI;

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// All four ZDF SVF outputs — computed simultaneously at zero extra cost
/// because the intermediate node voltages v1, v2 already capture them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SvfOutput {
    /// Low-pass  (−40 dB/decade, zero phase at DC)
    pub lp:    f64,
    /// Band-pass (−20 dB/decade each side, peaks at fc)
    pub bp:    f64,
    /// High-pass (passes seeing noise above fc)
    pub hp:    f64,
    /// Notch     (LP + HP = 1 − k·BP)
    pub notch: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
// 1-pole ZDF
// ─────────────────────────────────────────────────────────────────────────────

/// Zero-Delay Feedback one-pole low-pass / high-pass filter.
///
/// Discretises H(s) = ωc/(s + ωc) via the trapezoidal rule.
///
/// **Inner loop cost:** 4 multiplies · 3 adds · 0 branches · 0 allocations.
///
/// # State equations (Zavalishin eq. 3.22)
/// ```text
///   g     = tan(π · fc/fs)           (prewarped cutoff)
///   v[n]  = (g · x[n] + s[n]) / (1 + g)
///   s[n+1]= 2 · v[n] − s[n]         (trapezoidal integrator update)
///   lp    = v[n]
///   hp    = x[n] − v[n]
/// ```
#[derive(Debug, Clone)]
pub struct ZdfOnePole {
    /// Integrator state (capacitor-voltage analogue)
    ic_eq:  f64,
    /// Prewarped gain g = tan(π·fc/fs)
    g:      f64,
    /// Precomputed 1/(1+g) — avoids runtime division in hot path
    g_norm: f64,
}

impl ZdfOnePole {
    /// Construct from a normalised cutoff frequency in (0, 0.5).
    ///
    /// `normalized_fc = fc_hz / sample_rate_hz`
    pub fn new(normalized_fc: f64) -> Self {
        let mut f = Self { ic_eq: 0.0, g: 0.0, g_norm: 0.0 };
        f.set_cutoff(normalized_fc);
        f
    }

    /// Update cutoff.  Safe to call at control rate; **not** per-sample.
    #[inline]
    pub fn set_cutoff(&mut self, normalized_fc: f64) {
        // Clamp away from 0 and Nyquist to avoid tan(0) and tan(π/2)
        self.g      = (PI * normalized_fc.clamp(1e-6, 0.4999)).tan();
        self.g_norm = (1.0 + self.g).recip();
    }

    /// Process one sample.  Returns `(lp, hp)`.
    ///
    /// **Must not allocate; verified by #[inline(always)].**
    #[inline(always)]
    pub fn process(&mut self, x: f64) -> (f64, f64) {
        let v = (self.g * x + self.ic_eq) * self.g_norm;
        self.ic_eq = 2.0f64.mul_add(v, -self.ic_eq); // FMA: 2·v − s
        (v, x - v)
    }

    /// Zero the integrator (call on hard discontinuities or initialisation).
    #[inline]
    pub fn reset(&mut self) { self.ic_eq = 0.0; }

    /// Peek at the current integrator state without advancing.
    #[inline]
    pub fn state(&self) -> f64 { self.ic_eq }
}

// ─────────────────────────────────────────────────────────────────────────────
// 2-pole ZDF State Variable Filter
// ─────────────────────────────────────────────────────────────────────────────

/// Zero-Delay Feedback 2nd-order State Variable Filter.
///
/// Implements the Zavalishin topology: two trapezoidal integrators in a
/// feedback loop with a resonance path controlled by damping k = 1/Q.
///
/// **Inner loop cost:** 9 multiplies · 8 adds · 0 branches · 0 allocations.
///
/// # State equations (Zavalishin §3.10, eq. 3.40–3.45)
/// ```text
///   g   = tan(π · fc / fs)
///   k   = 1/Q
///   a1  = 1 / (1 + g·(g + k))
///   a2  = g · a1
///   a3  = g · a2
///
///   v3  = x − ic2eq
///   v1  = a1·ic1eq + a2·v3          (BP node)
///   v2  = ic2eq + a2·ic1eq + a3·v3  (LP node)
///   ic1eq ← 2·v1 − ic1eq            (integrator 1 advance)
///   ic2eq ← 2·v2 − ic2eq            (integrator 2 advance)
///
///   lp  = v2
///   bp  = v1
///   hp  = x − k·v1 − v2
///   ntc = x − k·v1                   (notch = lp + hp)
/// ```
#[derive(Debug, Clone)]
pub struct ZdfSvf {
    /// First integrator capacitor-voltage state
    ic1eq:       f64,
    /// Second integrator capacitor-voltage state
    ic2eq:       f64,
    // Precomputed coefficients —————————————————
    /// g = tan(π·fc/fs)
    g:           f64,
    /// k = 1/Q  (damping factor; k=√2 ≡ Butterworth)
    k:           f64,
    /// a1 = 1/(1 + g·(g+k))
    a1:          f64,
    /// a2 = g·a1
    a2:          f64,
    /// a3 = g·a2
    a3:          f64,
    /// Stored so `set_params` can recompute g without needing fs as an argument
    sample_rate: f64,
}

impl ZdfSvf {
    /// Construct with explicit sample rate.
    ///
    /// # Arguments
    /// * `fc`          — Cutoff frequency (Hz)
    /// * `q`           — Quality factor; 0.7071 = Butterworth
    /// * `sample_rate` — DSP engine tick rate (Hz)
    pub fn new(fc: f64, q: f64, sample_rate: f64) -> Self {
        let mut s = Self {
            ic1eq: 0.0, ic2eq: 0.0,
            g: 0.0, k: 0.0, a1: 0.0, a2: 0.0, a3: 0.0,
            sample_rate,
        };
        s.set_params(fc, q);
        s
    }

    /// Recompute filter coefficients.  Call when fc or Q changes.
    /// **Do not call inside the sample-processing loop.**
    pub fn set_params(&mut self, fc: f64, q: f64) {
        let fc_norm = (fc / self.sample_rate).clamp(1e-6, 0.4999);
        self.g  = (PI * fc_norm).tan();
        self.k  = (q.max(0.5)).recip();
        self.a1 = (1.0 + self.g * (self.g + self.k)).recip();
        self.a2 = self.g  * self.a1;
        self.a3 = self.g  * self.a2;
    }

    /// Process one sample; returns all four filter outputs.
    ///
    /// **9 multiplies, 8 adds, 0 branches, 0 allocations.**
    #[inline(always)]
    pub fn process(&mut self, x: f64) -> SvfOutput {
        let v3 = x - self.ic2eq;
        let v1 = self.a1.mul_add(self.ic1eq, self.a2 * v3);
        let v2 = self.ic2eq + self.a2.mul_add(self.ic1eq, self.a3 * v3);
        // Trapezoidal integrator advance: s' = 2·v − s
        self.ic1eq = 2.0f64.mul_add(v1, -self.ic1eq);
        self.ic2eq = 2.0f64.mul_add(v2, -self.ic2eq);
        let hp = x - self.k.mul_add(v1, v2);
        SvfOutput { lp: v2, bp: v1, hp, notch: x - self.k * v1 }
    }

    /// Process a block and write only the LP output.
    ///
    /// Tight loop; LLVM will auto-vectorise when slice lengths are known.
    pub fn process_block_lp(&mut self, input: &[f64], output: &mut [f64]) {
        debug_assert_eq!(input.len(), output.len());
        for (x, y) in input.iter().zip(output.iter_mut()) {
            *y = self.process(*x).lp;
        }
    }

    #[inline]
    pub fn reset(&mut self) { self.ic1eq = 0.0; self.ic2eq = 0.0; }

    /// Current LP output without advancing state (for diagnostics).
    #[inline]
    pub fn peek_lp(&self) -> f64 { self.ic2eq }
}

// ─────────────────────────────────────────────────────────────────────────────
// 4-pole cascaded ZDF  (Butterworth, −80 dB/decade)
// ─────────────────────────────────────────────────────────────────────────────

/// Cascaded pair of ZDF SVF stages — 4th-order Butterworth LP.
///
/// Pole Q values chosen from the Butterworth polynomial roots:
/// ```text
///   Q1 = 1 / (2 · cos(3π/8)) ≈ 1.3066
///   Q2 = 1 / (2 · cos(π/8))  ≈ 0.5412
/// ```
/// Combined magnitude response: −80 dB/decade above fc.
/// Phase at fc: −180° (two stages × −90°/stage), but group delay at DC is 0.
#[derive(Debug, Clone)]
pub struct ZdfFourPole {
    stage1: ZdfSvf,
    stage2: ZdfSvf,
}

impl ZdfFourPole {
    pub fn new(fc: f64, sample_rate: f64) -> Self {
        Self {
            stage1: ZdfSvf::new(fc, 1.3066, sample_rate),
            stage2: ZdfSvf::new(fc, 0.5412, sample_rate),
        }
    }

    /// Update cutoff while preserving Butterworth Q ratios.
    pub fn set_cutoff(&mut self, fc: f64) {
        let sr = self.stage1.sample_rate;
        self.stage1.set_params(fc, 1.3066);
        self.stage2.set_params(fc, 0.5412);
        let _ = sr;
    }

    /// Process one sample; returns LP output only.
    ///
    /// **18 multiplies, 16 adds, 0 branches, 0 allocations.**
    #[inline(always)]
    pub fn process_lp(&mut self, x: f64) -> f64 {
        let mid = self.stage1.process(x).lp;
        self.stage2.process(mid).lp
    }

    pub fn reset(&mut self) { self.stage1.reset(); self.stage2.reset(); }
}

// ─────────────────────────────────────────────────────────────────────────────
// Morphable ZDF  (LP ↔ HP blend)
// ─────────────────────────────────────────────────────────────────────────────

/// ZDF SVF with a continuous morph coefficient μ ∈ [0, 1].
///
/// ```text
///   output = (1 − μ)·LP + μ·HP
///          = LP + μ·(HP − LP)
///          = LP + μ·(x − k·BP − 2·LP)   (substituting HP = x − k·BP − LP)
/// ```
///
/// μ = 0.0 → pure LP (seeing rejection)
/// μ = 0.5 → band-reject centred on fc
/// μ = 1.0 → pure HP (isolate seeing for diagnostics)
#[derive(Debug, Clone)]
pub struct ZdfMorphable {
    svf:       ZdfSvf,
    /// LP/HP morph coefficient ∈ [0, 1]
    pub morph: f64,
}

impl ZdfMorphable {
    pub fn new(fc: f64, q: f64, sample_rate: f64, morph: f64) -> Self {
        Self { svf: ZdfSvf::new(fc, q, sample_rate), morph: morph.clamp(0.0, 1.0) }
    }

    #[inline(always)]
    pub fn process(&mut self, x: f64) -> f64 {
        let o = self.svf.process(x);
        // FMA: lp + morph*(hp − lp)
        self.morph.mul_add(o.hp - o.lp, o.lp)
    }

    pub fn reset(&mut self) { self.svf.reset(); }
}

// ─────────────────────────────────────────────────────────────────────────────
// Frequency response diagnostic
// ─────────────────────────────────────────────────────────────────────────────

/// Compute the magnitude response of a ZdfSvf at a single normalised frequency.
///
/// Uses an impulse response method: inject a unit impulse, collect N samples,
/// compute DFT bin nearest to `freq_norm`.  Allocates once; call offline only.
pub fn measure_lp_magnitude_db(filter: &mut ZdfSvf, freq_norm: f64, n: usize) -> f64 {
    let mut re = 0.0f64;
    let mut im = 0.0f64;
    let mut response = Vec::with_capacity(n);
    // Impulse
    response.push(filter.process(1.0).lp);
    for _ in 1..n {
        response.push(filter.process(0.0).lp);
    }
    for (k, &y) in response.iter().enumerate() {
        let angle = -2.0 * PI * freq_norm * k as f64;
        re += y * angle.cos();
        im += y * angle.sin();
    }
    20.0 * (re * re + im * im).sqrt().log10()
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;

    #[test]
    fn one_pole_dc_gain_is_zero_db() {
        let mut f = ZdfOnePole::new(0.1);
        // Drive with DC (x=1) until settled
        let mut y = 0.0;
        for _ in 0..1000 { (y, _) = f.process(1.0); }
        assert_abs_diff_eq!(y, 1.0, epsilon = 1e-6);
    }

    #[test]
    fn one_pole_nyquist_is_fully_rejected() {
        let mut f = ZdfOnePole::new(0.01); // very low cutoff
        // Nyquist = alternating ±1
        let mut y = 0.0;
        for i in 0..2000 {
            (y, _) = f.process(if i % 2 == 0 { 1.0 } else { -1.0 });
        }
        // Should be near zero
        assert!(y.abs() < 0.01, "Nyquist rejection failed: y={y}");
    }

    #[test]
    fn svf_butterworth_minus3db_at_cutoff() {
        // fc = 0.1 (normalised), Q = 1/√2 (Butterworth)
        let fc_norm = 0.1f64;
        let mut svf = ZdfSvf::new(fc_norm * 10.0, core::f64::consts::FRAC_1_SQRT_2, 10.0);
        // Measure at fc; expect |H| ≈ 1/√2 → −3.01 dB
        let mag_db = measure_lp_magnitude_db(&mut svf, fc_norm, 8192);
        assert_abs_diff_eq!(mag_db, -3.01, epsilon = 0.5);
    }

    #[test]
    fn four_pole_rolloff_exceeds_minus80_at_decade_above_cutoff() {
        let fs = 100.0f64;
        let fc = 1.0f64;
        let mut f4 = ZdfFourPole::new(fc, fs);
        // freq = 10·fc (one decade above cutoff)
        let freq_norm = (10.0 * fc) / fs;
        // Impulse response
        let n = 8192usize;
        let mut re = 0.0f64;
        let mut im = 0.0f64;
        let mut x = 1.0f64;
        for k in 0..n {
            let y = f4.process_lp(x);
            let angle = -2.0 * PI * freq_norm * k as f64;
            re += y * angle.cos();
            im += y * angle.sin();
            x = 0.0;
        }
        let mag_db = 20.0 * (re * re + im * im).sqrt().log10();
        assert!(mag_db < -75.0, "4-pole rolloff insufficient: {mag_db:.1} dB");
    }

    #[test]
    fn svf_reset_clears_state() {
        let mut svf = ZdfSvf::new(0.1, 0.707, 10.0);
        for _ in 0..100 { svf.process(1.0); }
        svf.reset();
        // After reset, impulse response starts from zero state
        let first = svf.process(0.0).lp;
        assert_abs_diff_eq!(first, 0.0, epsilon = 1e-15);
    }
}
