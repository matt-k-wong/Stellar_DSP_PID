//! Integration tests for the DSP subsystem.
//! These run via `cargo test --tests`.

use approx::assert_abs_diff_eq;
use stellar_dsp_pid::{
    config::{DspConfig, GuiderConfig, MountConfig},
    dsp::{AncEngine, ZdfFourPole, ZdfOnePole, ZdfSvf},
    GuideFrame,
};

fn dsp_cfg(fs: f64, fc: f64) -> DspConfig {
    DspConfig { sample_rate: fs, filter_cutoff_hz: fc, filter_q: 0.7071, oversample_ratio: 20 }
}
fn mount_cfg() -> MountConfig {
    MountConfig {
        worm_period_secs: 480.0, arcsec_per_ms_ra: 1.65, arcsec_per_ms_dec: 1.65,
        max_pulse_ms: 2000.0, min_pulse_ms: 1.0, aggressiveness: 1.0,
    }
}
fn guide_cfg() -> GuiderConfig {
    GuiderConfig {
        kp: 1.0, ki: 0.0, kd: 0.0, integral_clamp: 100.0,
        backlash_compensation_ms: 0.0, enable_pe_correction: false, pe_curve_path: None,
    }
}

// ── ZdfOnePole ──────────────────────────────────────────────────────────────

#[test]
fn one_pole_passband_gain_is_0db() {
    let mut f = ZdfOnePole::new(0.1); // fc = 0.1 normalised
    // DC settles to 1.0 for unit step input
    let mut y = 0.0;
    for _ in 0..5_000 { (y, _) = f.process(1.0); }
    assert_abs_diff_eq!(y, 1.0, epsilon = 1e-4);
}

#[test]
fn one_pole_stopband_attenuation() {
    let mut f = ZdfOnePole::new(0.01); // very low cutoff
    let mut out_rms = 0.0f64;
    let n = 10_000;
    for i in 0..n {
        // Inject Nyquist signal
        let x = if i % 2 == 0 { 1.0f64 } else { -1.0 };
        let (y, _) = f.process(x);
        out_rms += y * y;
    }
    out_rms = (out_rms / n as f64).sqrt();
    // Should be very small
    assert!(out_rms < 0.05, "stopband rms too large: {out_rms}");
}

// ── ZdfSvf ──────────────────────────────────────────────────────────────────

#[test]
fn svf_lp_hp_sum_equals_notch_plus_kbp() {
    // For a ZDF SVF: lp + hp = notch + k·bp (by construction from node equations)
    // We test: lp + hp ≈ x − k·bp
    let mut svf = ZdfSvf::new(1.0, 0.7071, 10.0);
    let x = 0.7;
    let o = svf.process(x);
    let k = 1.0 / 0.7071f64;
    let expected = x - k * o.bp;
    assert_abs_diff_eq!(o.lp + o.hp, expected, epsilon = 1e-12);
}

#[test]
fn four_pole_is_lower_than_two_pole_at_stopband() {
    let fs = 100.0f64;
    let fc = 1.0f64;
    let mut svf = ZdfSvf::new(fc, 0.7071, fs);
    let mut f4  = ZdfFourPole::new(fc, fs);
    // Inject at 10×fc
    let n = 20_000usize;
    let f_test = 10.0 * fc;
    let mut rms2 = 0.0f64;
    let mut rms4 = 0.0f64;
    for i in 0..n {
        let x = (2.0 * std::f64::consts::PI * f_test / fs * i as f64).sin();
        rms2 += svf.process(x).lp.powi(2);
        rms4 += f4.process_lp(x).powi(2);
    }
    rms2 = (rms2 / n as f64).sqrt();
    rms4 = (rms4 / n as f64).sqrt();
    assert!(rms4 < rms2 * 0.1, "4-pole should be much lower than 2-pole in stopband");
}

// ── AncEngine ───────────────────────────────────────────────────────────────

#[test]
fn anc_pe_correction_requires_phase_and_integrates_position_deltas() {
    let mut mount = mount_cfg();
    mount.arcsec_per_ms_ra = 0.007521;
    mount.min_pulse_ms = 0.0;
    let mut guide = guide_cfg();
    guide.enable_pe_correction = true;
    let mut engine = AncEngine::new(&dsp_cfg(10.0, 0.05), &mount, &guide);
    let amplitude = 5.0f64;
    let n = 4096usize;
    let pe: Vec<f64> = (0..n)
        .map(|i| amplitude * (2.0 * std::f64::consts::PI * i as f64 / n as f64).sin())
        .collect();
    engine.load_pe_curve(&pe);

    assert!(!engine.pe_table().is_enabled());
    assert_eq!(engine.tick().ra_west_ms, 0.0,
        "loading a curve alone must not arm PE output");

    // For this synthetic fixture, phase zero is a known reference. Over ten
    // ticks, integrated pulse displacement should equal the change in inverse
    // PE position, rather than sending the full position once per tick.
    engine.synchronize_pe_phase(0.0);
    assert!(engine.pe_table().is_enabled());
    let ticks = 10usize;
    let integrated_arcsec: f64 = (0..ticks)
        .map(|_| engine.tick().ra_west_ms * mount.arcsec_per_ms_ra)
        .sum();
    let expected_arcsec = -amplitude
        * (2.0 * std::f64::consts::PI * ticks as f64 / (480.0 * 10.0)).sin();
    assert!(
        (integrated_arcsec - expected_arcsec).abs() < 0.001,
        "integrated pulse displacement {integrated_arcsec:.6} arcsec should match per-cycle position delta {expected_arcsec:.6} arcsec"
    );
    assert!(integrated_arcsec < 0.0);

    // A new reset invalidates the phase reference and closes the output gate.
    engine.reset();
    assert!(!engine.pe_table().is_enabled());
    assert_eq!(engine.tick().ra_west_ms, 0.0);
}

#[test]
fn guide_pulse_respects_max_pulse_limit() {
    let mut mc = mount_cfg();
    mc.max_pulse_ms = 100.0;
    let mut engine = AncEngine::new(&dsp_cfg(10.0, 0.05), &mc, &guide_cfg());
    let frame = GuideFrame {
        timestamp: 0.0, ra_error_arcsec: 1000.0, dec_error_arcsec: 1000.0,
        star_flux: 50_000.0, valid: true,
    };
    engine.update_measurement(&frame);
    let pulse = engine.tick();
    assert!(pulse.ra_west_ms.abs() <= 100.0);
    assert!(pulse.dec_north_ms.abs() <= 100.0);
}
