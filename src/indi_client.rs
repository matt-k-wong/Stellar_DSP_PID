//! Minimal async INDI TCP client.
//!
//! INDI (Instrument Neutral Distributed Interface) is an XML-over-TCP
//! protocol for astronomy hardware control.  The server (`indiserver`)
//! listens on port 7624 and streams XML property updates.
//!
//! ## Protocol Summary
//!
//! ```text
//!  Client → Server:  <getProperties version="1.7"/>
//!  Server → Client:  <defNumberVector device="..." name="..."> ... </defNumberVector>
//!                    <setNumberVector device="..." name="..."> ... </setNumberVector>
//!  Client → Server:  <newNumberVector device="..." name="...">
//!                      <oneNumber name="TIMED_GUIDE_W">500</oneNumber>
//!                    </newNumberVector>
//! ```
//!
//! ## This Implementation
//!
//! - Connects to `host:port` via Tokio TCP.
//! - Spawns a background reader task that parses incoming XML into
//!   `IndiEvent` variants and sends them on an MPSC channel.
//! - Exposes a `send_pulse()` method that formats and sends guide pulse
//!   commands to the mount driver.
//!
//! XML parsing is intentionally lightweight: we only match the property
//! names we care about (guide star position and guide pulse acknowledgement)
//! and discard everything else.

use std::sync::Arc;

use anyhow::{Context, Result};
use log::{debug, error, info, warn};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::mpsc,
};

use crate::{GuideFrame, GuidePulse};

// ─────────────────────────────────────────────────────────────────────────────
// INDI event types
// ─────────────────────────────────────────────────────────────────────────────

/// Events emitted by the INDI reader task.
#[derive(Debug, Clone)]
pub enum IndiEvent {
    /// A guide star centroid update from the camera driver.
    GuideFrame(GuideFrame),
    /// Mount acknowledged a guide pulse (name, west_ms, north_ms).
    PulseAck { ra_west_ms: f64, dec_north_ms: f64 },
    /// An unrecognised but well-formed INDI property update.
    Other { device: String, name: String },
    /// TCP connection lost.
    Disconnected,
}

// ─────────────────────────────────────────────────────────────────────────────
// INDI XML builders
// ─────────────────────────────────────────────────────────────────────────────

/// Build the `<getProperties>` handshake message.
fn get_properties_xml() -> String {
    r#"<getProperties version="1.7"/>"#.to_owned() + "\n"
}

/// Build a `<newNumberVector>` message to send a guide pulse.
///
/// INDI mount drivers accept `TELESCOPE_TIMED_GUIDE_WE` and
/// `TELESCOPE_TIMED_GUIDE_NS`.  Pulse widths are in milliseconds.
/// Sending a value to `TIMED_GUIDE_W` drives the mount West; sending
/// to `TIMED_GUIDE_E` drives East (only one should be non-zero per command).
pub fn build_pulse_xml(device: &str, pulse: &GuidePulse) -> String {
    let mut xml = String::with_capacity(512);
    // RA axis ─────────────────────────────────────────────────────────────
    if pulse.ra_west_ms.abs() > f64::EPSILON {
        let (name, value) = if pulse.ra_west_ms > 0.0 {
            ("TIMED_GUIDE_W", pulse.ra_west_ms)
        } else {
            ("TIMED_GUIDE_E", -pulse.ra_west_ms)
        };
        xml.push_str(&format!(
            "<newNumberVector device=\"{device}\" name=\"TELESCOPE_TIMED_GUIDE_WE\">\
             <oneNumber name=\"{name}\">{value:.3}</oneNumber>\
             </newNumberVector>\n"
        ));
    }
    // DEC axis ────────────────────────────────────────────────────────────
    if pulse.dec_north_ms.abs() > f64::EPSILON {
        let (name, value) = if pulse.dec_north_ms > 0.0 {
            ("TIMED_GUIDE_N", pulse.dec_north_ms)
        } else {
            ("TIMED_GUIDE_S", -pulse.dec_north_ms)
        };
        xml.push_str(&format!(
            "<newNumberVector device=\"{device}\" name=\"TELESCOPE_TIMED_GUIDE_NS\">\
             <oneNumber name=\"{name}\">{value:.3}</oneNumber>\
             </newNumberVector>\n"
        ));
    }
    xml
}

// ─────────────────────────────────────────────────────────────────────────────
// XML line parser (minimal, alloc-on-event but not in hot path)
// ─────────────────────────────────────────────────────────────────────────────

/// Minimal line-oriented INDI XML parser.
///
/// INDI is a streaming XML protocol; we can't use a document parser.
/// Instead we use a simple state machine that accumulates lines between
/// `<setNumberVector` / `<defNumberVector` open tags and closing `</...>`
/// tags, then extracts values with substring matching.
#[derive(Default)]
struct IndiParser {
    /// Buffer accumulating the current property block
    buffer:      String,
    /// Device name extracted from the opening tag
    device:      String,
    /// Property name extracted from the opening tag
    prop_name:   String,
    /// True when inside a NumberVector block we care about
    in_block:    bool,
    /// Accumulated `<oneNumber>` values
    values:      Vec<(String, f64)>,
}

impl IndiParser {
    /// Feed one line.  Returns `Some(IndiEvent)` when a complete block is parsed.
    fn feed_line(&mut self, line: &str) -> Option<IndiEvent> {
        let trimmed = line.trim();

        // ── Opening tag: start of a NumberVector ──────────────────────────
        if !self.in_block {
            if let Some(rest) = trimmed
                .strip_prefix("<setNumberVector")
                .or_else(|| trimmed.strip_prefix("<defNumberVector"))
            {
                self.device    = extract_attr(rest, "device").unwrap_or_default();
                self.prop_name = extract_attr(rest, "name").unwrap_or_default();
                self.buffer.clear();
                self.values.clear();
                self.in_block  = true;
            }
            return None;
        }

        // ── Inside a block: collect oneNumber elements ────────────────────
        if trimmed.starts_with("<oneNumber") {
            if let Some(name) = extract_attr(trimmed, "name") {
                // Value is between > and </oneNumber>
                if let (Some(start), Some(end)) = (trimmed.find('>'), trimmed.find("</")) {
                    if start < end {
                        let val_str = trimmed[start + 1..end].trim();
                        if let Ok(v) = val_str.parse::<f64>() {
                            self.values.push((name, v));
                        }
                    }
                }
            }
        }

        // ── Closing tag: emit event ───────────────────────────────────────
        if trimmed.starts_with("</setNumberVector") || trimmed.starts_with("</defNumberVector") {
            self.in_block = false;
            return self.emit_event();
        }

        // Inline-closed tag e.g. <setNumberVector .../> — rare in INDI but handle it
        if self.in_block && trimmed.ends_with("/>") && trimmed.starts_with("<setNumberVector") {
            self.in_block = false;
            return self.emit_event();
        }

        None
    }

    fn emit_event(&mut self) -> Option<IndiEvent> {
        match self.prop_name.as_str() {
            // Guide star position from camera driver
            "GUIDE_STAR_POSITION" | "RAPID_GUIDE_DATA" => {
                let x = self.values.iter().find(|(n, _)| n.contains("X") || n.contains("RA"))
                            .map(|(_, v)| *v).unwrap_or(0.0);
                let y = self.values.iter().find(|(n, _)| n.contains("Y") || n.contains("DEC"))
                            .map(|(_, v)| *v).unwrap_or(0.0);
                let flux = self.values.iter().find(|(n, _)| n.contains("FLUX") || n.contains("SNR"))
                            .map(|(_, v)| *v).unwrap_or(10_000.0);
                let frame = GuideFrame {
                    timestamp:        now_secs(),
                    ra_error_arcsec:  x,
                    dec_error_arcsec: y,
                    star_flux:        flux,
                    valid:            true,
                };
                Some(IndiEvent::GuideFrame(frame))
            }
            // Mount acknowledged guide pulse
            "TELESCOPE_TIMED_GUIDE_WE" => {
                let w = self.values.iter().find(|(n, _)| n == "TIMED_GUIDE_W").map(|(_, v)| *v).unwrap_or(0.0);
                let e = self.values.iter().find(|(n, _)| n == "TIMED_GUIDE_E").map(|(_, v)| *v).unwrap_or(0.0);
                Some(IndiEvent::PulseAck { ra_west_ms: w - e, dec_north_ms: 0.0 })
            }
            _ => Some(IndiEvent::Other {
                device:   self.device.clone(),
                name:     self.prop_name.clone(),
            }),
        }
    }
}

/// Extract a named attribute value from an XML tag fragment.
/// e.g. `extract_attr(r#"device="Camera""#, "device")` → `Some("Camera")`
fn extract_attr(s: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let start  = s.find(&needle)? + needle.len();
    let end    = s[start..].find('"')? + start;
    Some(s[start..end].to_owned())
}

/// Current time as seconds since UNIX epoch (platform-dependent).
fn now_secs() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ─────────────────────────────────────────────────────────────────────────────
// IndiClient
// ─────────────────────────────────────────────────────────────────────────────

/// Async INDI TCP client.
///
/// Spawns an internal reader task; exposes channels for events and pulse sends.
pub struct IndiClient {
    /// Channel to receive parsed INDI events
    pub event_rx:    mpsc::Receiver<IndiEvent>,
    /// Channel to send raw XML strings to the server
    xml_tx:          mpsc::Sender<String>,
    mount_device:    Arc<String>,
}

impl IndiClient {
    /// Connect to `indiserver` and start the background reader task.
    ///
    /// # Arguments
    /// * `host`         — Hostname or IP
    /// * `port`         — INDI port (default 7624)
    /// * `mount_device` — INDI device name for the mount driver
    pub async fn connect(host: &str, port: u16, mount_device: &str) -> Result<Self> {
        let addr = format!("{host}:{port}");
        info!("Connecting to indiserver at {addr}");
        let stream = TcpStream::connect(&addr)
            .await
            .with_context(|| format!("Failed to connect to indiserver at {addr}"))?;
        info!("Connected to indiserver");

        let (tcp_read, mut tcp_write) = stream.into_split();
        let (event_tx, event_rx)     = mpsc::channel::<IndiEvent>(64);
        let (xml_tx,   mut xml_rx)   = mpsc::channel::<String>(64);

        // ── Writer task: relay XML strings to TCP ─────────────────────────
        tokio::spawn(async move {
            // Send handshake
            if let Err(e) = tcp_write.write_all(get_properties_xml().as_bytes()).await {
                error!("INDI write error (handshake): {e}");
                return;
            }
            while let Some(msg) = xml_rx.recv().await {
                if let Err(e) = tcp_write.write_all(msg.as_bytes()).await {
                    error!("INDI write error: {e}");
                    break;
                }
                debug!("INDI TX: {}", msg.trim());
            }
        });

        // ── Reader task: parse XML and emit events ────────────────────────
        let event_tx_clone = event_tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(tcp_read);
            let mut line   = String::new();
            let mut parser = IndiParser::default();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => {
                        warn!("INDI server closed connection");
                        let _ = event_tx_clone.send(IndiEvent::Disconnected).await;
                        break;
                    }
                    Ok(_) => {
                        debug!("INDI RX: {}", line.trim());
                        if let Some(event) = parser.feed_line(&line) {
                            match &event {
                                IndiEvent::Other { .. } => { /* suppress noise */ }
                                _ => {
                                    if event_tx_clone.send(event).await.is_err() { break; }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("INDI read error: {e}");
                        let _ = event_tx_clone.send(IndiEvent::Disconnected).await;
                        break;
                    }
                }
            }
        });

        Ok(Self {
            event_rx,
            xml_tx,
            mount_device: Arc::new(mount_device.to_owned()),
        })
    }

    /// Send a guide pulse to the mount driver.
    ///
    /// Returns immediately; the INDI write task handles the actual TCP write.
    pub async fn send_pulse(&self, pulse: &GuidePulse) -> Result<()> {
        let xml = build_pulse_xml(&self.mount_device, pulse);
        if xml.is_empty() { return Ok(()); }
        self.xml_tx.send(xml).await.context("INDI XML channel closed")
    }

    /// Send raw XML (for advanced use / testing).
    pub async fn send_raw(&self, xml: String) -> Result<()> {
        self.xml_tx.send(xml).await.context("INDI XML channel closed")
    }
}
