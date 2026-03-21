//! Stellar DSP PID — Binary entry point.
//!
//! Parses CLI arguments, loads configuration, and drives the guiding loop.

use anyhow::Result;
use clap::{Parser, Subcommand};
use log::info;
use tokio::sync::oneshot;

use stellar_dsp_pid::{config::Config, guider::Guider};

// ─────────────────────────────────────────────────────────────────────────────
// CLI
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(
    name  = "stellar-dsp-pid",
    about = "Feed-forward DSP guiding engine for astrophotography mounts",
    version,
    author,
)]
struct Cli {
    /// Path to TOML configuration file
    #[arg(short, long, default_value = "config/default.toml")]
    config: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run the active guiding loop (default)
    Guide {
        /// Optional path to a saved PE curve JSON file (overrides config)
        #[arg(long)]
        pe_curve: Option<String>,
    },
    /// Record raw guide star measurements for PE calibration.
    /// Saves a pe_curve.json after N complete worm cycles.
    Calibrate {
        /// Number of worm cycles to record
        #[arg(long, default_value = "3")]
        cycles: u32,
        /// Output path for the PE curve JSON
        #[arg(long, default_value = "config/pe_curve.json")]
        output: String,
    },
    /// Print DSP filter frequency response to stdout (CSV: hz,db)
    Freqplot {
        /// Number of frequency points
        #[arg(long, default_value = "256")]
        points: usize,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// Tokio main
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    ).init();

    let cli = Cli::parse();
    info!("Stellar DSP PID v{}", env!("CARGO_PKG_VERSION"));

    let config = Config::from_toml_file(&cli.config)?;
    info!("Loaded config from {}", cli.config);

    match cli.command.unwrap_or(Commands::Guide { pe_curve: None }) {
        Commands::Guide { pe_curve } => {
            run_guide(config, pe_curve).await?;
        }
        Commands::Calibrate { cycles, output } => {
            run_calibrate(config, cycles, output).await?;
        }
        Commands::Freqplot { points } => {
            run_freqplot(&config, points);
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Guide mode
// ─────────────────────────────────────────────────────────────────────────────

async fn run_guide(config: Config, pe_curve_override: Option<String>) -> Result<()> {
    let mut guider = Guider::new(config.clone()).await?;

    // Load PE curve: CLI override takes precedence, then config file, then skip
    let pe_path = pe_curve_override
        .or_else(|| config.guider.pe_curve_path.clone());
    if let Some(path) = pe_path {
        if let Err(e) = guider.load_pe_curve_from_file(&path) {
            log::warn!("Could not load PE curve from {path}: {e} — PE correction disabled");
        }
    }

    // Ctrl-C → graceful shutdown
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Ctrl-C received — shutting down");
        let _ = shutdown_tx.send(());
    });

    guider.run(shutdown_rx).await
}

// ─────────────────────────────────────────────────────────────────────────────
// Calibration mode
// ─────────────────────────────────────────────────────────────────────────────

async fn run_calibrate(config: Config, cycles: u32, output: String) -> Result<()> {
    use stellar_dsp_pid::indi_client::{IndiClient, IndiEvent};

    info!("Calibration mode: recording {cycles} worm cycle(s) into {output}");
    let worm_period = config.mount.worm_period_secs;
    let duration    = Duration::from_secs_f64(worm_period * cycles as f64 + 60.0);

    let mut indi = IndiClient::connect(
        &config.indi.host,
        config.indi.port,
        &config.indi.mount_device,
    ).await?;

    let deadline = tokio::time::Instant::now() + duration;
    let mut samples: Vec<(f64, f64)> = Vec::with_capacity(
        (worm_period * cycles as f64 * config.dsp.sample_rate) as usize,
    );

    info!("Recording for {:.0} s ...", duration.as_secs_f64());
    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            Some(event) = indi.event_rx.recv() => {
                if let IndiEvent::GuideFrame(f) = event {
                    if f.valid {
                        samples.push((f.timestamp, f.ra_error_arcsec));
                    }
                }
            }
            _ = tokio::time::sleep_until(deadline) => { break; }
        }
    }
    info!("Collected {} samples", samples.len());

    // Build PE table and serialise the measured curve
    use stellar_dsp_pid::dsp::PeriodicErrorTable;
    let mut table: PeriodicErrorTable<4096> = PeriodicErrorTable::new(
        worm_period,
        config.dsp.sample_rate,
        config.mount.arcsec_per_ms_ra,
    );
    table.calibrate_from_samples(&samples);
    let curve_json = serde_json::to_string_pretty(&table.to_vec())?;
    std::fs::write(&output, curve_json)?;
    info!("PE curve written to {output}  (rms={:.2}\")", table.rms_arcsec);
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Frequency response plot (diagnostic)
// ─────────────────────────────────────────────────────────────────────────────

fn run_freqplot(config: &Config, points: usize) {
    use stellar_dsp_pid::dsp::zdf_filter::{measure_lp_magnitude_db, ZdfSvf};
    let fs = config.dsp.sample_rate;
    let fc = config.dsp.filter_cutoff_hz;
    let q  = config.dsp.filter_q;

    println!("hz,db");
    for i in 0..points {
        // Log-spaced from 0.001 to Nyquist
        let f_norm = 0.001 * (0.499 / 0.001f64).powf(i as f64 / (points - 1) as f64);
        let f_hz   = f_norm * fs;
        let mut svf = ZdfSvf::new(fc, q, fs);
        let db = measure_lp_magnitude_db(&mut svf, f_norm, 32_768);
        println!("{f_hz:.6},{db:.4}");
    }
}

use std::time::Duration;
