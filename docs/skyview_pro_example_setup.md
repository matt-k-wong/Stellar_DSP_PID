# Illustrative SkyView Pro / SynScan first setup

This is a generic example for the SkyView Pro family with a SynScan V3
handset and SynScan Wi-Fi adapter. It is not an inventory or status report for
any particular person's equipment. The profile values are worked examples or
conditional estimates; verify the actual mount, controller, network path, and
driver before use. Keep serial numbers, personal network reports, Wi-Fi names,
and photos with identifying details in private notes.

## 1. Inspect the example offline

From the repository root:

```sh
cargo run -- --config config/profiles/skyview-pro-example.toml inspect
```

`inspect` parses and prints the configuration. It does not connect to INDI,
discover hardware, or send mount commands. For a different mount, copy
`config/default.toml` and record measured values and their sources in a local
profile.

## 2. Identify the actual hardware

Record the model and revision shown on the mount head, motor controller,
handset, and Wi-Fi adapter. Treat each item separately: a manual/catalog number
is not a serial number, a radio module certification label is not the adapter's
model or firmware, and handset firmware is not motor-controller hardware
revision. Keep exact serials and identifying photographs in private records.

On a SynScan V3 handset, open
`Utility Func > Show Information > Version`, press ENTER, and scroll through
the fields. The manual lists H.C. Firmware, Database, H.C. Hardware, Motor
Controller, and H.C. Serial #. It defines Database as the handset database
version and Motor Controller as the motor-controller firmware version. The
Motor Controller field is available when the handset is connected to the
controller box. To check guide rate, open `SETUP > Auto Guide Speed` and record
the displayed value without changing it. See the official
[SynScan V3 hand-control manual](https://inter-static.skywatcher.com/upfiles/en_download_caty01415653149.pdf).

Use the manual for the exact mount/controller combination to confirm the
worm-wheel tooth count and PEC details. Do not infer a revision from a family
name or open the gear housing solely to count teeth.

## 3. Discover the connection path

Candidate direct-adapter route: INDI `indi_eqmod_telescope` / `EQMod Mount`,
Network or Ethernet transport, UDP port 11880. A common hotspot example is
`192.168.4.1`; in station mode, use the adapter's assigned address. The INDI
server is a separate endpoint, commonly TCP port 7624. The SynScan app route
through `indi_synscan_telescope` uses TCP port 11882 and is a different
connection path. These routes and defaults are examples, not confirmation of
compatibility or guide-pulse support for a particular mount/controller.

For a local Mac report, connect to the adapter and run:

```sh
python3 scripts/probe_synscan_wifi.py
```

The script saves a timestamped report under `~/Downloads`. It records the
current network name, local address, and route, then sends only documented
read-only inquiries (`:e1`, `:a1`, and `:s1`) to the selected adapter target.
It does not send motion or settings commands. Choose a target explicitly for
station mode, for example `--host <adapter-ip> --port 11880`. For local network
details only, use `--network-only`. Keep the generated report private; do not
copy its SSID, addresses, or other local network details into public profiles.
The inquiry definitions are in the official
[Sky-Watcher Basic API source](https://github.com/skywatcher-pacific/skywatcher_open/blob/master/BasicApi_CS/Mount_Skywatcher.cs).
For a normal reply, the report decodes only the exact form `=` followed by six
hex digits and a carriage return. It applies Sky-Watcher's little-endian
`BCDstr2long` conversion; `:e` uses the source's motor-board version byte
reordering, while `:a` and `:s` are labeled as controller microsteps per axis
revolution (`GetMicrostepsPerRevolution`) and per worm revolution
(`GetMicrostepsPerWormRevolution`), respectively, following the
[INDI Sky-Watcher API](https://github.com/indilib/indi/blob/master/drivers/telescope/skywatcherAPI.cpp)
and [field definitions](https://github.com/indilib/indi/blob/master/drivers/telescope/skywatcherAPI.h).
When both `:a1` and `:s1` have valid positive replies, the report shows their
ratio as possible evidence for an approximate worm-wheel tooth count. That
ratio is not a direct physical tooth count or a measured worm period. The raw
ASCII and hex reply stay in the report; malformed, error, and missing replies
remain inconclusive.

A timeout means no UDP reply was received; it does not establish
incompatibility. A reply confirms only that an inquiry received a response. It
does not prove GoTo, tracking, or pulse-guiding support. Inspect the exact
driver's properties before relying on it, and use one mount controller at a
time. Do not operate the handset/app and INDI concurrently.

## 4. Treat example numbers as estimates

The example profile includes two calculations, neither measured on a physical
mount:

- **598.36 s** assumes an RA wheel with 144 teeth: sidereal day / 144. The
  actual wheel tooth count and worm period vary by equipment. A SkyView Pro
  manual's 480 s PEC-training interval is not a measured worm period.
- **0.007521 arcsec/ms** is an ideal 0.5× sidereal rate at Dec 0, not a selected
  rate or measured response for any particular setup. For guide rate `g` times
  sidereal, the ideal Dec-0 scale is `15.04157 × g / 1000 arcsec/ms`.

Read the selected guide rate from the handset/driver and keep that value
separate from the measured mount displacement per millisecond. Measure pulse
response independently in each axis and direction with a verified centroid
scale and axis transform. RA image-plane response also varies with declination.
Keep feedback aggressiveness at zero until those measurements are available.

Periodic-error feed-forward remains disabled. A measured curve still needs a
physical worm-phase reference before playback; a plausible period or a loaded
curve alone is not enough to synchronize it.

## 5. Camera and measurement requirements

No camera is configured in the example. To estimate guide-image scale, use the
guide optic's focal length `f` in millimetres and camera pixel pitch `p` in
micrometres:

```text
arcsec/pixel = 206.265 × p / f
```

A guide camera and rigid guide optic need a usable star at the chosen exposure
cadence. Pixel scale converts image displacement to an angle but does not
convert camera X/Y into signed RA/Dec errors. That also needs a reference
centroid and a measured camera-to-mount axis rotation and sign.

The project's INDI measurement device must publish `GUIDE_ERROR_ARCSEC` with
signed sky-axis `RA_ERROR_ARCSEC`, `DEC_ERROR_ARCSEC`, positive `STAR_FLUX`, and
`CAPTURE_UNIX_SECONDS` from the source capture time. The timestamp must use
Unix seconds, increase strictly for each new capture, and use a clock
synchronized with the guider host. Frames with future, repeated, or out-of-order
timestamps, and captures older than `max(3 × guide_exposure, 5 seconds)`, are
rejected. Republishing a cached frame does not refresh its age; capture age is
rechecked before every pulse.
Ordinary camera drivers provide images or pixels; a centroid bridge must
convert them to this contract. The bridge is not included in the repository.
`indi.camera_device` names the device publishing that measurement contract and
may differ from the physical camera's device name.

## 6. Readiness boundary

`inspect` is the camera-free offline discovery path. Before guiding, identify
the mount and measurement devices, verify the sky-axis conversion, and measure
pulse response. The guide loop fails closed without a usable measurement
stream. PE playback also remains blocked until a measured curve can be aligned
to physical worm phase.

For the reusable workflow across other equipment, see the
[equipment discovery and gear profile guide](gear_profile_discovery.md). For
the INDI measurement contract, see [INDI setup](indi_setup.md) and
[calibration notes](calibration_guide.md).
