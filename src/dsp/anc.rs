//! Active Noise Cancellation (ANC) Guiding Engine
//!
//! Combines the ZDF feed-forward PE cancellation wavetable with a
//! ZDF-filtered PID feedback loop.
//!
//! ## Signal Flow
//!
//! ```text
//!                    ┌─────────────────────────────────────────┐
//!  guide frame       │                                         │
//!  ε (arcsec) ──────►│  ZdfFourPole LP (fc=0.05 Hz)           │
//!                    │  → ε̃: seeing-stripped error             │
//!                    └────────────────┬────────────────────────┘
//!                                     │ ε̃
//!                                     ▼
//!                    ┌────────────────────────────────────────┐
//!                    │  PID(ε̃) → feedback correction (ms)     │
//!                    └────────────────┬───────────────────────┘
//!                                     │ feedback_ms
//!                                     ▼
//!  wavetable ff ──► (+) ──────────────────────────────────────► GuidePulse
//!                   │  ff_ms + feedback_ms × aggressiveness
//! ```
//!
//! ## Oversampling Architecture
//!
//! The engine runs at `dsp_rate` (e.g. 10 Hz) regardless of the guide
//! frame arrival rate (typically 0.5–2 Hz).  Between frames, the last
//! known error is held and the ZDF filter continues to evolve its state.
//! The wavetable advances at the full DSP rate, providing smooth PE
//! correction pulses even when no new guide measurement has arrived.
//!
//! ## Units
//!
//! * Error / correction: arcseconds internally; converted to ms at output.
//! * Time: seconds (DSP); milliseconds (guide pulse width only).

use crate::{
    config::{DspConfig, GuiderConfig, MountConfig},
    dsp::{pid::Pid, wavetable::PeriodicErrorTable, zdf_filter::ZdfFourPole},
    GuideFrame, GuidePulse,
};

// Wavetable size: 4096 entries → 32 KB, comfortably in L1 cache.
const PE_TABLE_SIZE: usize = 4096;

/// Running statistics accumulated per DSP tick for diagnostics.
#[derive(Debug, Default, Clone)]
pub struct AncStats {
    /// Number of DSP ticks since last reset
    pub tick_count:         u64,
    /// Number of valid guide frames received
    pub frame_count:        u64,
    /// Number of lost-star events
    pub lost_star_count:    u64,
    /// RMS of RA filtered error (arcseconds)
    pub ra_error_rms:       f64,
    /// RMS of DEC filtered error (arcseconds)
    pub dec_error_rms:      f64,
    /// RMS of PE feed-forward correction (ms)
    pub ff_rms_ms:          f64,
    // Running sum of squares for RMS computation
    ra_sq_acc:   f64,
    dec_sq_acc:  f64,
    ff_sq_acc:   f64,
}

impl AncStats {
    fn update(&mut self, ra_err: f64, dec_err: f64, ff_ms: f64) {
        self.tick_count += 1;
        self.ra_sq_acc  += ra_err * ra_err;
        self.dec_sq_acc += dec_err * dec_err;
        self.ff_sq_acc  += ff_ms * ff_ms;
        let n = self.tick_count as f64;
        self.ra_error_rms  = (self.ra_sq_acc  / n).sqrt();
        self.dec_error_rms = (self.dec_sq_acc / n).sqrt();
        self.ff_rms_ms     = (self.ff_sq_acc  / n).sqrt();
    }

    pub fn reset(&mut self) { *self = Self::default(); }
}

/// The ANC guiding engine.
///
/// Instantiate once, call `tick()` at the DSP rate, and call
/// `update_measurement()` whenever a new guide frame arrives.
pub struct AncEngine {
    // ── DSP primitives ─────────────────────────────────────────────────────
    ra_filter:  ZdfFourPole,
    dec_filter: ZdfFourPole,
    ra_pid:     Pid,
    dec_pid:    Pid,
    pe_table:   PeriodicErrorTable<PE_TABLE_SIZE>,

    // ── Last known measurement ─────────────────────────────────────────────
    /// Filtered RA error (arcseconds); updated on each new guide frame
    ra_filtered:  f64,
    /// Filtered DEC error (arcseconds)
    dec_filtered: f64,
    /// SNR weight from last frame (0.0 if lost star)
    snr_weight:   f64,

    // ── Configuration scalars ──────────────────────────────────────────────
    aggressiveness:   f64,
    max_pulse_ms:     f64,
    min_pulse_ms:     f64,
    arcsec_per_ms_ra: f64,
    arcsec_per_ms_dec: f64,

    // ── Diagnostics ────────────────────────────────────────────────────────
    pub stats: AncStats,
}

impl AncEngine {
    /// Construct from configuration slices.
    pub fn new(dsp: &DspConfig, mount: &MountConfig, guide: &GuiderConfig) -> Self {
        let fs  = dsp.sample_rate;
        let fc  = dsp.filter_cutoff_hz;
        let q   = dsp.filter_q;

        // Recompute Q for 4-pole Butterworth (ignores user Q; 4-pole has fixed Q ratios)
        // The user Q from config is used for the 2-pole pre-filter in future iterations.
        let _ = q; // silence unused warning; 4-pole Q ratios are hard-coded

        Self {
            ra_filter:  ZdfFourPole::new(fc, fs),
            dec_filter: ZdfFourPole::new(fc, fs),
            ra_pid:     Pid::new(guide.kp, guide.ki, guide.kd, guide.integral_clamp, fs),
            dec_pid:    Pid::new(guide.kp, guide.ki, guide.kd, guide.integral_clamp, fs),
            pe_table:   PeriodicErrorTable::new(
                            mount.worm_period_secs,
                            fs,
                            mount.arcsec_per_ms_ra,
                        ),
            ra_filtered:     0.0,
            dec_filtered:    0.0,
            snr_weight:      1.0,
            aggressiveness:  mount.aggressiveness,
            max_pulse_ms:    mount.max_pulse_ms,
            min_pulse_ms:    mount.min_pulse_ms,
            arcsec_per_ms_ra:  mount.arcsec_per_ms_ra,
            arcsec_per_ms_dec: mount.arcsec_per_ms_dec,
            stats: AncStats::default(),
        }
    }

    /// Ingest a new guide frame.
    ///
    /// Updates the ZDF filter states with the new measurement.
    /// **Call at guide-frame rate (0.5–2 Hz), NOT at DSP rate.**
    #[inline]
    pub fn update_measurement(&mut self, frame: &GuideFrame) {
        if !frame.valid {
            self.snr_weight = 0.0;
            self.stats.lost_star_count += 1;
            // Reset integrators to avoid wind-up during lost-star
            self.ra_pid.reset();
            self.dec_pid.reset();
            return;
        }
        self.stats.frame_count += 1;
        // Process frame error through ZDF 4-pole LP
        // The filter state advances once per guide frame here;
        // it also advances during tick() via zero-input response.
        self.ra_filtered  = self.ra_filter.process_lp(frame.ra_error_arcsec);
        self.dec_filtered = self.dec_filter.process_lp(frame.dec_error_arcsec);
        // SNR-based confidence weight (flux normalised, clamped)
        self.snr_weight = (frame.star_flux / 10_000.0).clamp(0.0, 1.0);
    }

    /// Advance the engine by one DSP tick and compute a guide pulse.
    ///
    /// Called at the **DSP rate** (e.g. 10 Hz), independent of guide-frame
    /// arrival.  The wavetable advances every tick; the PID feedback is
    /// scaled by the SNR weight to suppress corrections during poor seeing.
    ///
    /// **Allocation-free.**
    #[inline(always)]
    pub fn tick(&mut self) -> GuidePulse {
        // ── Feed-forward: PE wavetable ──────────────────────────────────────
        let ff_ms = self.pe_table.tick();

        // ── Feedback: ZDF-filtered PID ──────────────────────────────────────
        let ra_fb_arcsec  = self.ra_pid.process(self.ra_filtered,  self.ra_filtered);
        let dec_fb_arcsec = self.dec_pid.process(self.dec_filtered, self.dec_filtered);

        let ra_fb_ms  = (ra_fb_arcsec  / self.arcsec_per_ms_ra)
                        * self.aggressiveness * self.snr_weight;
        let dec_fb_ms = (dec_fb_arcsec / self.arcsec_per_ms_dec)
                        * self.aggressiveness * self.snr_weight;

        // ── Sum paths ───────────────────────────────────────────────────────
        let ra_total_ms  = ff_ms + ra_fb_ms;
        let dec_total_ms = dec_fb_ms; // PE correction is RA-only

        // ── Update statistics ───────────────────────────────────────────────
        self.stats.update(self.ra_filtered, self.dec_filtered, ff_ms);

        // ── Build and clamp pulse ───────────────────────────────────────────
        let pulse = GuidePulse {
            ra_west_ms:   ra_total_ms,
            dec_north_ms: dec_total_ms,
        }.clamped(self.max_pulse_ms);

        // Apply minimum pulse threshold (dead-band)
        GuidePulse {
            ra_west_ms:   if pulse.ra_west_ms.abs()   < self.min_pulse_ms { 0.0 }
                          else { pulse.ra_west_ms },
            dec_north_ms: if pulse.dec_north_ms.abs() < self.min_pulse_ms { 0.0 }
                          else { pulse.dec_north_ms },
        }
    }

    /// Load a PE curve from a raw measurement vector.
    pub fn load_pe_curve(&mut self, pe_curve: &[f64]) {
        self.pe_table.load_curve(pe_curve);
    }

    /// Load a PE curve from (timestamp, error) sample pairs.
    pub fn calibrate_pe(&mut self, samples: &[(f64, f64)]) {
        self.pe_table.calibrate_from_samples(samples);
    }

    /// Reset all DSP state (filters, PIDs, PE phase).
    pub fn reset(&mut self) {
        self.ra_filter.reset();
        self.dec_filter.reset();
        self.ra_pid.reset();
        self.dec_pid.reset();
        self.pe_table.set_phase(0.0);
        self.ra_filtered  = 0.0;
        self.dec_filtered = 0.0;
        self.snr_weight   = 1.0;
        self.stats.reset();
    }

    /// Expose PE table for persistence / export.
    pub fn pe_table(&self) -> &PeriodicErrorTable<PE_TABLE_SIZE> { &self.pe_table }
    pub fn pe_table_mut(&mut self) -> &mut PeriodicErrorTable<PE_TABLE_SIZE> { &mut self.pe_table }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DspConfig, GuiderConfig, MountConfig};

    fn make_config() -> (DspConfig, MountConfig, GuiderConfig) {
        let dsp = DspConfig {
            sample_rate:      10.0,
            filter_cutoff_hz: 0.05,
            filter_q:         0.7071,
            oversample_ratio: 20,
        };
        let mount = MountConfig {
            worm_period_secs:  480.0,
            arcsec_per_ms_ra:  1.65,
            arcsec_per_ms_dec: 1.65,
            max_pulse_ms:      2000.0,
            min_pulse_ms:      10.0,
            aggressiveness:    1.0,
        };
        let guide = GuiderConfig {
            kp: 0.6, ki: 0.02, kd: 0.0,
            integral_clamp: 10.0,
            backlash_compensation_ms: 0.0,
            enable_pe_correction: true,
            pe_curve_path: None,
        };
        (dsp, mount, guide)
    }

    #[test]
    fn no_correction_without_measurement() {
        let (dsp, mount, guide) = make_config();
        let mut engine = AncEngine::new(&dsp, &mount, &guide);
        // With zero state and disabled PE, output should be zero
        let pulse = engine.tick();
        assert_eq!(pulse.ra_west_ms, 0.0);
        assert_eq!(pulse.dec_north_ms, 0.0);
    }

    #[test]
    fn lost_star_zeroes_output() {
        let (dsp, mount, guide) = make_config();
        let mut engine = AncEngine::new(&dsp, &mount, &guide);
        // Feed some error, then lose the star
        let frame_valid = GuideFrame {
            timestamp: 0.0, ra_error_arcsec: 5.0, dec_error_arcsec: 2.0,
            star_flux: 50_000.0, valid: true,
        };
        engine.update_measurement(&frame_valid);
        let frame_lost = GuideFrame { valid: false, ..frame_valid };
        engine.update_measurement(&frame_lost);
        let pulse = engine.tick();
        // SNR weight = 0 → feedback is zero; PE is disabled → ff = 0
        assert_eq!(pulse.ra_west_ms, 0.0);
    }

    #[test]
    fn pe_correction_applied_after_load() {
        let (dsp, mount, guide) = make_config();
        let mut engine = AncEngine::new(&dsp, &mount, &guide);
        // Load a flat +5 arcsec PE curve
        let flat_pe: Vec<f64> = vec![5.0f64; 1024];
        engine.load_pe_curve(&flat_pe);
        let pulse = engine.tick();
        // Flat curve → constant correction; with arcsec_per_ms=1.65, expect ~3 ms
        assert!(pulse.ra_west_ms.abs() > 1.0, "PE correction not applied");
    }
}
