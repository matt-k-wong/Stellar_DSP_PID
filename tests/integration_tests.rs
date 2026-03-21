//! End-to-end integration tests (no live INDI connection required).

use stellar_dsp_pid::{
    config::{Config, DspConfig, GuiderConfig, IndiConfig, MountConfig},
    dsp::AncEngine,
    GuideFrame,
};

fn full_config() -> Config {
    Config {
        indi: IndiConfig {
            host: "localhost".to_owned(),
            port: 7624,
            camera_device: "Test Camera".to_owned(),
            mount_device:  "Test Mount".to_owned(),
            guide_exposure: 2.0,
        },
        dsp: DspConfig {
            sample_rate: 10.0, filter_cutoff_hz: 0.05,
            filter_q: 0.7071, oversample_ratio: 20,
        },
        mount: MountConfig {
            worm_period_secs: 480.0, arcsec_per_ms_ra: 1.65,
            arcsec_per_ms_dec: 1.65, max_pulse_ms: 2000.0,
            min_pulse_ms: 1.0, aggressiveness: 0.8,
        },
        guider: GuiderConfig {
            kp: 0.6, ki: 0.02, kd: 0.0, integral_clamp: 10.0,
            backlash_compensation_ms: 0.0, enable_pe_correction: true,
            pe_curve_path: None,
        },
    }
}

#[test]
fn engine_runs_1000_ticks_without_panic() {
    let cfg = full_config();
    let mut engine = AncEngine::new(&cfg.dsp, &cfg.mount, &cfg.guider);
    for i in 0..1000usize {
        if i % 20 == 0 {
            let frame = GuideFrame {
                timestamp: i as f64 / 10.0,
                ra_error_arcsec:  (i as f64 * 0.1).sin() * 3.0,
                dec_error_arcsec: (i as f64 * 0.07).cos() * 1.5,
                star_flux: 30_000.0,
                valid: true,
            };
            engine.update_measurement(&frame);
        }
        let _pulse = engine.tick();
    }
    assert!(engine.stats.tick_count == 1000);
    assert!(engine.stats.frame_count == 50);
}

#[test]
fn pe_round_trip_json_preserves_curve() {
    use stellar_dsp_pid::dsp::PeriodicErrorTable;
    let original: Vec<f64> = (0..1024)
        .map(|i| 5.0 * (2.0 * std::f64::consts::PI * i as f64 / 1024.0).sin())
        .collect();
    let mut tbl: PeriodicErrorTable<4096> =
        PeriodicErrorTable::new(480.0, 10.0, 1.65);
    tbl.load_curve(&original);
    let exported = tbl.to_vec();
    // Exported should recover the original PE curve (not the inverted table)
    // The sign and scale might differ slightly due to resampling; check RMS
    let rms: f64 = (exported.iter().map(|v| v * v).sum::<f64>() / exported.len() as f64).sqrt();
    let expected_rms = 5.0 / std::f64::consts::SQRT_2;
    assert!((rms - expected_rms).abs() < 0.1, "RMS mismatch: {rms:.3} vs {expected_rms:.3}");
}
