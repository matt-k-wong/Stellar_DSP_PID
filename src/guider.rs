//! Top-level guiding state machine.
//!
//! Owns an `AncEngine` and an `IndiClient`, drives the DSP tick loop at
//! `sample_rate` Hz, and feeds INDI guide frames into the engine when they
//! arrive on the event channel.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, ensure, Result};
use log::{error, info, warn};
use tokio::time;

use crate::{
    config::Config,
    dsp::AncEngine,
    indi_client::{IndiClient, IndiEvent},
    GuideFrame,
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
    last_capture_timestamp: Option<f64>,
}

impl Guider {
    /// Construct from a validated `Config`.  Connects to indiserver.
    pub async fn new(config: Config) -> Result<Self> {
        ensure!(
            !config.indi.mount_device.trim().is_empty(),
            "guiding requires an identified INDI mount device; run inspect and configure indi.mount_device first"
        );
        ensure!(
            !config.indi.camera_device.trim().is_empty(),
            "guiding requires an INDI measurement source that publishes verified sky-axis arcseconds and CAPTURE_UNIX_SECONDS; set indi.camera_device to the centroid bridge device when available"
        );
        ensure!(
            config.mount.aggressiveness > 0.0,
            "guiding remains disarmed while mount.aggressiveness is 0; measure guide pulse response before enabling feedback"
        );
        if config.guider.enable_pe_correction {
            bail!(
                "PE playback is not armed: this build has no mount worm-phase synchronization input; a measured curve alone cannot establish phase"
            );
        }
        let indi = IndiClient::connect(
            &config.indi.host,
            config.indi.port,
            &config.indi.mount_device,
            &config.indi.camera_device,
        ).await?;

        let engine = AncEngine::new(&config.dsp, &config.mount, &config.guider);

        Ok(Self {
            config,
            engine,
            indi,
            state: GuiderState::WaitingForStar,
            last_capture_timestamp: None,
        })
    }

    /// Load a pre-measured PE curve from a JSON file.
    pub fn load_pe_curve_from_file(&mut self, path: &str) -> Result<()> {
        ensure!(
            self.config.guider.enable_pe_correction,
            "PE curve load requested while guider.pe_correction is disabled"
        );
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
        info!(
            "Waiting for {}. No mount correction is sent until a fresh timestamped measurement arrives.",
            "GUIDE_ERROR_ARCSEC from the configured measurement/bridge device"
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
                let Some(now) = unix_now_secs() else {
                    self.reject_measurement(&frame, "system clock is unavailable");
                    return;
                };
                if !frame.timestamp.is_finite() {
                    self.reject_measurement(&frame, "capture timestamp is not finite");
                    return;
                }
                if frame.timestamp > now {
                    self.reject_measurement(&frame, "capture timestamp is in the future");
                    return;
                }
                if now - frame.timestamp > self.measurement_max_age().as_secs_f64() {
                    self.reject_measurement(&frame, "capture is stale");
                    return;
                }
                if self
                    .last_capture_timestamp
                    .is_some_and(|last| frame.timestamp <= last)
                {
                    self.reject_measurement(&frame, "capture timestamp is not increasing");
                    return;
                }

                self.last_capture_timestamp = Some(frame.timestamp);
                if !frame.valid {
                    self.engine.update_measurement(&frame);
                    self.state = GuiderState::LostStar;
                    return;
                }
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

    fn reject_measurement(&mut self, frame: &GuideFrame, reason: &str) {
        warn!("Guide measurement rejected ({reason}); pausing mount corrections");
        self.engine.update_measurement(&GuideFrame {
            valid: false,
            ..*frame
        });
        self.state = GuiderState::LostStar;
    }

    fn measurement_max_age(&self) -> Duration {
        Duration::from_secs_f64((self.config.indi.guide_exposure * 3.0).max(5.0))
    }

    async fn dsp_tick(&mut self) {
        if self.state != GuiderState::Guiding { return; }

        let capture_is_current = match (self.last_capture_timestamp, unix_now_secs()) {
            (Some(capture_time), Some(now)) => {
                capture_time.is_finite()
                    && capture_time <= now
                    && now - capture_time <= self.measurement_max_age().as_secs_f64()
            }
            _ => false,
        };
        if !capture_is_current {
            warn!("Latest guide capture is stale or has an invalid clock; pausing mount corrections");
            self.engine.update_measurement(&GuideFrame {
                timestamp: self.last_capture_timestamp.unwrap_or(0.0),
                ra_error_arcsec: 0.0,
                dec_error_arcsec: 0.0,
                star_flux: 0.0,
                valid: false,
            });
            self.state = GuiderState::LostStar;
            return;
        }

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

fn unix_now_secs() -> Option<f64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs_f64())
}
