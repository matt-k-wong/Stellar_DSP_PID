//! Criterion benchmarks for DSP hot paths.
//! Run: cargo bench

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

use stellar_dsp_pid::{
    config::{DspConfig, GuiderConfig, MountConfig},
    dsp::{
        wavetable::PeriodicErrorTable,
        zdf_filter::{ZdfFourPole, ZdfOnePole, ZdfSvf},
        AncEngine,
    },
    GuideFrame,
};

// ── ZDF one-pole ─────────────────────────────────────────────────────────────

fn bench_zdf_one_pole(c: &mut Criterion) {
    let mut group = c.benchmark_group("zdf_one_pole");
    group.throughput(Throughput::Elements(1024));
    group.bench_function("1024_samples", |b| {
        let mut f   = ZdfOnePole::new(0.1);
        let input   = vec![0.5f64; 1024];
        let mut out = vec![0.0f64; 1024];
        b.iter(|| {
            for (x, y) in input.iter().zip(out.iter_mut()) {
                let (lp, _) = f.process(black_box(*x));
                *y = lp;
            }
            black_box(&out);
        });
    });
    group.finish();
}

// ── ZDF SVF ───────────────────────────────────────────────────────────────────

fn bench_zdf_svf_block(c: &mut Criterion) {
    let mut group = c.benchmark_group("zdf_svf");
    group.throughput(Throughput::Elements(4096));
    group.bench_function("4096_block_lp", |b| {
        let mut svf = ZdfSvf::new(0.05, 0.7071, 10.0);
        let input   = vec![0.5f64; 4096];
        let mut out = vec![0.0f64; 4096];
        b.iter(|| {
            svf.process_block_lp(black_box(&input), &mut out);
            black_box(&out);
        });
    });
    group.finish();
}

// ── ZDF 4-pole ────────────────────────────────────────────────────────────────

fn bench_zdf_four_pole(c: &mut Criterion) {
    let mut group = c.benchmark_group("zdf_four_pole");
    group.throughput(Throughput::Elements(1024));
    group.bench_function("1024_scalar", |b| {
        let mut f4 = ZdfFourPole::new(0.05, 10.0);
        let input  = vec![0.5f64; 1024];
        b.iter(|| {
            let mut acc = 0.0f64;
            for &x in &input {
                acc += f4.process_lp(black_box(x));
            }
            black_box(acc);
        });
    });
    group.finish();
}

// ── Wavetable tick ────────────────────────────────────────────────────────────

fn bench_wavetable_tick(c: &mut Criterion) {
    let mut group = c.benchmark_group("wavetable");
    group.throughput(Throughput::Elements(4800)); // 1 worm cycle at 10 Hz

    group.bench_function("4800_ticks_4096pt", |b| {
        let pe: Vec<f64> = (0..1024)
            .map(|i| 8.0 * (2.0 * std::f64::consts::PI * i as f64 / 1024.0).sin())
            .collect();
        let mut tbl: PeriodicErrorTable<4096> =
            PeriodicErrorTable::new(480.0, 10.0, 1.65);
        tbl.load_curve(&pe);
        b.iter(|| {
            let mut acc = 0.0f64;
            for _ in 0..4800 { acc += tbl.tick(); }
            black_box(acc);
        });
    });
    group.finish();
}

// ── Full ANC engine tick ──────────────────────────────────────────────────────

fn bench_anc_tick(c: &mut Criterion) {
    let dsp_cfg = DspConfig { sample_rate: 10.0, filter_cutoff_hz: 0.05,
                              filter_q: 0.7071, oversample_ratio: 20 };
    let mnt_cfg = MountConfig { worm_period_secs: 480.0, arcsec_per_ms_ra: 1.65,
                                arcsec_per_ms_dec: 1.65, max_pulse_ms: 2000.0,
                                min_pulse_ms: 1.0, aggressiveness: 0.8 };
    let gui_cfg = GuiderConfig { kp: 0.6, ki: 0.02, kd: 0.0, integral_clamp: 10.0,
                                  backlash_compensation_ms: 0.0,
                                  enable_pe_correction: true, pe_curve_path: None };

    c.benchmark_group("anc_engine")
        .throughput(Throughput::Elements(4800))
        .bench_function("4800_ticks_with_pe", |b| {
            let mut engine = AncEngine::new(&dsp_cfg, &mnt_cfg, &gui_cfg);
            let pe: Vec<f64> = (0..1024)
                .map(|i| 8.0 * (2.0 * std::f64::consts::PI * i as f64 / 1024.0).sin())
                .collect();
            engine.load_pe_curve(&pe);
            let frame = GuideFrame {
                timestamp: 0.0, ra_error_arcsec: 1.5, dec_error_arcsec: 0.5,
                star_flux: 40_000.0, valid: true,
            };
            engine.update_measurement(&frame);

            b.iter(|| {
                for _ in 0..4800 { black_box(engine.tick()); }
            });
        });
}

criterion_group!(
    benches,
    bench_zdf_one_pole,
    bench_zdf_svf_block,
    bench_zdf_four_pole,
    bench_wavetable_tick,
    bench_anc_tick,
);
criterion_main!(benches);
