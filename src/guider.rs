//! Top-level guiding state machine.
//!
//! Owns an `AncEngine` and an `IndiClient`, drives the DSP tick loop at
//! `sample_rate` Hz, and feeds INDI guide frames into the engine when they
//! arrive on the event channel.

use std::time::Duration;

use anyhow::Result;
use log::{error, info, warn};
use tokio::time;

use crate::{
    config::Config,
    dsp::AncEngine,
    indi_client::{IndiClient, IndiEvent},
    GuideFrame, GuidePulse,
};

/// State machine states for the guiding loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiderState {
    /// Waiting for the first valid guide frame
    WaitingForStar,
    /// Actively guiding
    Guiding,
    /// Star lost; holding last correction; waiting for recovery
    LostStar,
    /// User-requested pause
    Paused,
    /// Shutting down
    Stopped,
}

/// The top-level guider: binds INDI events to the ANC DSP engine.
pub struct Guider {
    config: Config,
    engine: AncEngine,
    indi:   IndiClient,
    state:  GuiderState,
}

impl Guider {
    /// Construct from a validated `Config`.  Connects to indiserver.
    pub async fn new(config: Config) -> Result<Self> {
        let indi = IndiClient::connect(
            &config.indi.host,
            config.indi.port,
            &config.indi.mount_device,
        ).await?;

        let engine = AncEngine::new(&config.dsp, &config.mount, &config.guider);

        Ok(Self {
            config,
            engine,
            indi,
            state: GuiderState::WaitingForStar,
        })
    }

    /// Load a pre-measured PE curve from a JSON file.
    pub fn load_pe_curve_from_file(&mut self, path: &str) -> Result<()> {
        let text  = std::fs::read_to_string(path)?;
        let curve: Vec<f64> = serde_json::from_str(&text)?;
        self.engine.load_pe_curve(&curve);
        info!("Loaded PE curve ({} points) from {path}", curve.len());
        Ok(())
    }

    /// Run the guiding loop until `shutdown_rx` fires or an unrecoverable
    /// error occurs.
    pub async fn run(
        &mut self,
        mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<()> {
        let tick_interval = Duration::from_secs_f64(self.config.dsp.sample_rate.recip());
        let mut ticker    = time::interval(tick_interval);
        // Avoid burst after sleep
        ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

        info!(
            "Guider started — DSP rate: {} Hz, fc: {} Hz",
            self.config.dsp.sample_rate,
            self.config.dsp.filter_cutoff_hz,
        );

        loop {
            tokio::select! {
                biased;

                // ── Shutdown signal ────────────────────────────────────
                _ = &mut shutdown_rx => {
                    info!("Shutdown signal received");
                    self.state = GuiderState::Stopped;
                    break;
                }

                // ── Incoming INDI event ────────────────────────────────
                Some(event) = self.indi.event_rx.recv() => {
                    self.handle_indi_event(event).await;
                }

                // ── DSP tick ───────────────────────────────────────────
                _ = ticker.tick() => {
                    self.dsp_tick().await;
                }
            }
        }
        // Print final statistics
        let s = &self.engine.stats;
        info!(
            "Guide session ended. Frames: {}, Lost-star: {}, RA rms: {:.2}\" DEC rms: {:.2}\"",
            s.frame_count, s.lost_star_count, s.ra_error_rms, s.dec_error_rms,
        );
        Ok(())
    }

    async fn handle_indi_event(&mut self, event: IndiEvent) {
        match event {
            IndiEvent::GuideFrame(frame) => {
                match self.state {
                    GuiderState::WaitingForStar | GuiderState::LostStar => {
                        info!("Star acquired (flux={:.0})", frame.star_flux);
                        self.state = GuiderState::Guiding;
                    }
                    _ => {}
                }
                self.engine.update_measurement(&frame);
            }
            IndiEvent::Disconnected => {
                error!("Lost connection to indiserver");
                self.state = GuiderState::Stopped;
            }
            IndiEvent::PulseAck { ra_west_ms, dec_north_ms } => {
                // Could log round-trip latency here
                let _ = (ra_west_ms, dec_north_ms);
            }
            _ => {}
        }
    }

    async fn dsp_tick(&mut self) {
        if self.state != GuiderState::Guiding { return; }

        let pulse = self.engine.tick();

        // Dead-band: skip pulse if both axes are below minimum
        if pulse.ra_west_ms.abs() < f64::EPSILON
            && pulse.dec_north_ms.abs() < f64::EPSILON
        {
            return;
        }

        if let Err(e) = self.indi.send_pulse(&pulse).await {
            warn!("Failed to send guide pulse: {e}");
        }
    }
}
