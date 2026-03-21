//! Discrete-time PID controller with derivative filtering and anti-windup.
//!
//! Used as the **feedback** path in the ANC engine; its input is the
//! ZDF-filtered star position error ε̃ (arcseconds), and its output is an
//! additional correction in arcseconds (converted to ms at the ANC layer).
//!
//! ## Design Notes
//!
//! **Derivative on measurement (not error):** `Kd * d/dt(−y)` instead of
//! `Kd * d/dt(e)`.  This avoids the derivative kick when the setpoint
//! changes abruptly (e.g., a new guide frame arrives after a lost-star event).
//!
//! **Derivative low-pass filter (ZDF one-pole):** The raw derivative term
//! is processed through a ZDF LP at fc_deriv = 0.3·fs before being added.
//! This suppresses amplification of quantisation noise from the centroid.
//!
//! **Conditional integration (anti-windup):** Integrator is frozen when
//! |u_total| ≥ integral_clamp.  Classic back-calculation anti-windup is
//! overkill for the slow guide-loop timescales here.
//!
//! **Trapezoidal integration of error:** The integral is accumulated using
//! the trapezoidal rule (average of current and previous error) to match
//! the integration method used by the ZDF filters in the signal path.

use crate::dsp::zdf_filter::ZdfOnePole;

/// PID controller tuned for star-position error loops.
#[derive(Debug, Clone)]
pub struct Pid {
    /// Proportional gain (dimensionless)
    pub kp: f64,
    /// Integral gain (1/s → multiply by dt to get per-sample gain)
    pub ki: f64,
    /// Derivative gain (s → divided by dt to get per-sample gain)
    pub kd: f64,

    /// Integrator accumulator (arcseconds)
    integral:      f64,
    /// Integral magnitude clamp (arcseconds)
    integral_clamp: f64,

    /// Previous error for trapezoidal integral and derivative
    prev_error: f64,
    /// Previous measurement for derivative-on-measurement
    prev_meas:  f64,

    /// ZDF one-pole LP on the derivative term (fc = 0.3·Nyquist)
    deriv_filter: ZdfOnePole,

    /// Sample period dt = 1/fs (seconds)
    dt: f64,
}

impl Pid {
    /// Construct with explicit parameters.
    ///
    /// # Arguments
    /// * `kp`, `ki`, `kd`   — PID gains
    /// * `integral_clamp`   — Maximum |integrator| magnitude (arcseconds)
    /// * `sample_rate`      — DSP engine tick rate (Hz)
    pub fn new(kp: f64, ki: f64, kd: f64, integral_clamp: f64, sample_rate: f64) -> Self {
        // Derivative filter cutoff: 0.3 × Nyquist normalised
        let deriv_fc_norm = 0.30_f64;
        Self {
            kp,
            ki,
            kd,
            integral: 0.0,
            integral_clamp,
            prev_error: 0.0,
            prev_meas:  0.0,
            deriv_filter: ZdfOnePole::new(deriv_fc_norm),
            dt: sample_rate.recip(),
        }
    }

    /// Process one sample.
    ///
    /// # Arguments
    /// * `error`       — Setpoint minus measurement (arcseconds)
    /// * `measurement` — Raw guide star position error (for D-on-meas)
    ///
    /// # Returns
    /// PID output in arcseconds.  Sign: positive = West / North correction.
    ///
    /// **Allocation-free.**
    #[inline(always)]
    pub fn process(&mut self, error: f64, measurement: f64) -> f64 {
        // ── Integral (trapezoidal rule) ────────────────────────────────────
        let integral_delta = 0.5 * self.dt * (error + self.prev_error);
        let proposed_integral = self.integral + integral_delta;
        // Anti-windup: clamp absolute value
        self.integral = proposed_integral.clamp(-self.integral_clamp, self.integral_clamp);

        // ── Derivative on measurement (not error) ─────────────────────────
        // d/dt(-measurement) ≈ -(measurement[n] − measurement[n-1]) / dt
        let raw_deriv = -(measurement - self.prev_meas) / self.dt;
        // Low-pass filter derivative to suppress noise amplification
        let (filtered_deriv, _) = self.deriv_filter.process(raw_deriv);

        // ── State update ──────────────────────────────────────────────────
        self.prev_error = error;
        self.prev_meas  = measurement;

        // ── Output ────────────────────────────────────────────────────────
        self.kp * error + self.ki * self.integral + self.kd * filtered_deriv
    }

    /// Hard-reset all state (call after lost-star events).
    pub fn reset(&mut self) {
        self.integral   = 0.0;
        self.prev_error = 0.0;
        self.prev_meas  = 0.0;
        self.deriv_filter.reset();
    }

    /// Update gains at runtime (safe; takes effect on next `process` call).
    pub fn set_gains(&mut self, kp: f64, ki: f64, kd: f64) {
        self.kp = kp;
        self.ki = ki;
        self.kd = kd;
    }

    /// Read the current integrator state (arcseconds).
    pub fn integral(&self) -> f64 { self.integral }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;

    #[test]
    fn p_only_proportional_response() {
        let mut pid = Pid::new(1.0, 0.0, 0.0, 100.0, 10.0);
        let out = pid.process(3.0, 0.0);
        assert_abs_diff_eq!(out, 3.0, epsilon = 1e-10);
    }

    #[test]
    fn integral_accumulates_over_time() {
        let fs = 10.0f64;
        let mut pid = Pid::new(0.0, 1.0, 0.0, 1000.0, fs);
        // Apply constant error of 1.0 for 10 samples (1 second)
        let mut out = 0.0;
        for _ in 0..10 { out = pid.process(1.0, 0.0); }
        // Integral of 1.0 over 1.0 s = 1.0; Ki=1 → output ≈ 1.0
        assert_abs_diff_eq!(out, 1.0, epsilon = 0.05);
    }

    #[test]
    fn anti_windup_clamps_integral() {
        let mut pid = Pid::new(0.0, 1.0, 0.0, 5.0, 10.0);
        // Drive for a long time with large error
        for _ in 0..10_000 { pid.process(10.0, 0.0); }
        assert!(pid.integral().abs() <= 5.0 + 1e-9);
    }

    #[test]
    fn reset_clears_state() {
        let mut pid = Pid::new(1.0, 1.0, 0.0, 100.0, 10.0);
        for _ in 0..100 { pid.process(5.0, 5.0); }
        pid.reset();
        let out = pid.process(0.0, 0.0);
        assert_abs_diff_eq!(out, 0.0, epsilon = 1e-10);
    }
}
