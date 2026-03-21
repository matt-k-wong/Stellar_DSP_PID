//! Configuration structures deserialised from TOML.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub indi:   IndiConfig,
    pub dsp:    DspConfig,
    pub mount:  MountConfig,
    pub guider: GuiderConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndiConfig {
    #[serde(default = "default_host")]
    pub host:          String,
    #[serde(default = "default_port")]
    pub port:          u16,
    pub camera_device: String,
    pub mount_device:  String,
    /// Guide camera exposure time (seconds)
    pub guide_exposure: f64,
}

fn default_host() -> String { "localhost".to_owned() }
fn default_port() -> u16    { 7624 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DspConfig {
    /// Internal DSP tick rate (Hz) — oversample the guide cadence
    pub sample_rate:      f64,
    /// ZDF LP filter cutoff frequency (Hz)
    pub filter_cutoff_hz: f64,
    /// ZDF filter Q factor
    pub filter_q:         f64,
    /// Oversampling ratio (may be computed from sample_rate / (1/guide_exposure))
    #[serde(default = "default_oversample")]
    pub oversample_ratio: u32,
}

fn default_oversample() -> u32 { 20 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountConfig {
    /// Worm gear period (seconds). 0 disables PE correction.
    pub worm_period_secs:  f64,
    /// Arcseconds per millisecond of RA guide pulse
    pub arcsec_per_ms_ra:  f64,
    /// Arcseconds per millisecond of DEC guide pulse
    pub arcsec_per_ms_dec: f64,
    /// Upper pulse duration limit (ms)
    pub max_pulse_ms:      f64,
    /// Lower pulse duration threshold (ms); below this, skip
    pub min_pulse_ms:      f64,
    /// Global aggressiveness scalar [0.0, 1.0]
    pub aggressiveness:    f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiderConfig {
    pub kp:                      f64,
    pub ki:                      f64,
    pub kd:                      f64,
    /// Maximum integrator magnitude (arcseconds)
    pub integral_clamp:          f64,
    /// DEC backlash pre-pulse (ms)
    pub backlash_compensation_ms: f64,
    /// Enable wavetable PE feed-forward
    pub enable_pe_correction:    bool,
    /// Optional path to a saved PE curve JSON file
    #[serde(default)]
    pub pe_curve_path:           Option<String>,
}

impl Config {
    pub fn from_toml_file(path: &str) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.dsp.sample_rate > 0.0, "dsp.sample_rate must be positive");
        anyhow::ensure!(self.dsp.filter_cutoff_hz > 0.0
            && self.dsp.filter_cutoff_hz < self.dsp.sample_rate / 2.0,
            "filter_cutoff_hz must be in (0, sample_rate/2)");
        anyhow::ensure!(self.dsp.filter_q >= 0.5, "filter_q must be >= 0.5");
        anyhow::ensure!(self.mount.max_pulse_ms > self.mount.min_pulse_ms,
            "max_pulse_ms must exceed min_pulse_ms");
        anyhow::ensure!(self.mount.aggressiveness >= 0.0
            && self.mount.aggressiveness <= 1.0,
            "aggressiveness must be in [0, 1]");
        Ok(())
    }
}
