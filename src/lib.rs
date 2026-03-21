//! # Stellar DSP PID — Library Crate
//!
//! Exports the DSP engine, INDI client, and guiding state machine for
//! use in tests, benchmarks, and the binary entry-point.

pub mod config;
pub mod dsp;
pub mod guider;
pub mod indi_client;

/// Guide axis identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis { Ra, Dec }

/// A single guide measurement arriving from the INDI stream.
#[derive(Debug, Clone, Copy)]
pub struct GuideFrame {
    /// Timestamp (seconds since UNIX epoch)
    pub timestamp: f64,
    /// RA tracking error (arcseconds, positive = East)
    pub ra_error_arcsec: f64,
    /// DEC tracking error (arcseconds, positive = North)
    pub dec_error_arcsec: f64,
    /// Star flux — used for SNR weighting
    pub star_flux: f64,
    /// True if star was successfully centroided
    pub valid: bool,
}

/// A guide pulse command to send to the mount via INDI.
#[derive(Debug, Clone, Copy, Default)]
pub struct GuidePulse {
    /// West correction (ms); East is negative of this.
    pub ra_west_ms: f64,
    /// North correction (ms); South is negative of this.
    pub dec_north_ms: f64,
}

impl GuidePulse {
    /// Clamp both axes to `max_ms`.
    #[inline]
    pub fn clamped(self, max_ms: f64) -> Self {
        Self {
            ra_west_ms:   self.ra_west_ms.clamp(-max_ms, max_ms),
            dec_north_ms: self.dec_north_ms.clamp(-max_ms, max_ms),
        }
    }
}
