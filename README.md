# Stellar DSP PID

An experimental Rust DSP guider for telescope mounts controlled through INDI.
The project combines filtered PID feedback with an optional periodic-error
feed-forward table. It is not a plug-and-play guiding application: a camera
centroid bridge, measured pulse response, and mount-specific configuration are
required before guide mode can run.

## Start without a camera

The repository includes a reusable generic configuration and an illustrative
SkyView Pro / SynScan V3 profile. The example is not an inventory of anyone's
equipment; its profile values are placeholders or conditional estimates.
Inspect either profile offline:

```sh
cargo run -- --config config/default.toml inspect
cargo run -- --config config/profiles/skyview-pro-example.toml inspect
```

`inspect` reads the TOML file and reports assumptions. It does not connect to
INDI, check hardware, or send mount commands. The illustrative SkyView setup
guide explains how to identify a mount, handset, and adapter, and which values
need measurement: [docs/skyview_pro_example_setup.md](docs/skyview_pro_example_setup.md).
The guide also includes an offline Mac script for collecting the adapter's
local Wi-Fi route and, optionally, raw replies to read-only inquiries: [probe_synscan_wifi.py](scripts/probe_synscan_wifi.py).
With no subcommand, the CLI runs this offline inspection. Start mount guiding
only with the explicit `guide` subcommand.

## Profiles

Profiles are ordinary TOML files selected with `--config`. Use
`config/default.toml` as a generic template or copy a file into
`config/profiles/` for a mount and controller combination. Mark every period,
guide rate, driver name, and camera as either measured, manufacturer-listed,
calculated, or unverified in its comments. See the
[equipment discovery and profile guide](docs/gear_profile_discovery.md) for a
reusable inventory, measurement, and provenance workflow.

The SkyView example leaves the camera device empty, PE disabled, and
aggressiveness at zero. It records two conditional calculations:

- A 598.36 s worm period assuming a 144-tooth wheel. This is not measured and
  differs from the SkyView Pro manual's 480 s PEC-training interval, which is
  not itself a measured worm period.
- A 0.007521 arcsec/ms ideal guide scale for 0.5× sidereal at Dec 0. This is a
  theoretical example, not a selected rate or measured pulse response.

The profile's `EQMod Mount` device name and direct Wi-Fi adapter route are
candidates only. Pulse-guiding support for a specific SynScan V3 / SkyView Pro
setup must be verified. See [INDI setup](docs/indi_setup.md).

## Camera and measurement requirements

The guider accepts measurements only from the configured INDI measurement
device's `GUIDE_ERROR_ARCSEC` property, containing signed `RA_ERROR_ARCSEC`,
`DEC_ERROR_ARCSEC`, positive `STAR_FLUX`, and `CAPTURE_UNIX_SECONDS` values.
The timestamp must be the source capture time in Unix seconds, not the INDI
message arrival time. These error values must already be converted to sky-axis
arcseconds. Raw `GUIDE_STAR_POSITION` and
`RAPID_GUIDE_DATA` pixel coordinates are ignored because they need a reference
centroid, image scale, and camera-to-mount axis transform.

For pixel pitch `p` in micrometres and guide focal length `f` in millimetres:

```text
image scale = 206.265 × p / f  arcsec/pixel
```

`indi.camera_device` names the device publishing this property. It may name a
future centroid bridge and can differ from the physical guide camera. Ordinary
camera drivers expose images or pixels and do not publish this project-specific
property. The project does not include the centroiding bridge yet. Guide mode
also requires identified mount and measurement source devices, nonzero tuned
aggressiveness, and fresh measurements. Capture timestamps must increase
strictly and not be in the future. Captures older than
`max(3 × guide_exposure, 5 seconds)` are rejected. Repeated cached samples do
not renew freshness, and guiding pauses as soon as the last accepted capture
exceeds that age.
See [calibration notes](docs/calibration_guide.md).

## Periodic error

PE correction is disabled in the supplied profiles. A measured curve must be
synchronized to a physical worm-phase reference before playback; this app does
not yet receive that reference. Setting a period or loading a curve alone is
not enough. The `calibrate` command requires a configured measurement source and
verified arcsecond measurements, and its timestamp-binned output is not
replay-ready without phase synchronization.

## Commands

```sh
# Offline profile inspection
cargo run -- --config config/profiles/skyview-pro-example.toml inspect

# Guide only after hardware, camera conversion, pulse response, and settings
# are verified and configured
cargo run -- --config path/to/profile.toml guide

# Capture raw measurements for later analysis; this does not send guide pulses
cargo run -- --config path/to/profile.toml calibrate --cycles 6 \
  --output config/pe_curve.json

# Print the DSP filter response
cargo run -- --config path/to/profile.toml freqplot --points 256
```

## Implementation layout

```text
src/config.rs          TOML profiles and configuration validation
src/main.rs            CLI, offline inspect, capture and guide commands
src/guider.rs          INDI event handling and fail-closed guide state machine
src/indi_client.rs     INDI TCP client and verified measurement property parser
src/dsp/anc.rs         PID/ANC engine
src/dsp/wavetable.rs   Phase-synchronized periodic-error table
config/default.toml    Generic safe template
config/profiles/       Gear-specific profile files
docs/                  Setup, INDI, theory, and calibration notes
```

## Build

Requires Rust 1.75 or newer. The library and CLI can be built with:

```sh
cargo build --release
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for project contribution guidance.

## License

MIT — see [LICENSE](LICENSE).
