# Equipment discovery and gear profiles

Use this guide to identify a mount and its supporting equipment before
choosing a profile or tuning the guider. Keep the generic template generic.
Keep actual personal equipment observations in private notes unless publication
of those specific facts is explicitly authorized; use clearly illustrative
profiles for public examples. The [SkyView Pro first-setup guide](skyview_pro_example_setup.md)
is an example for one older SynScan system, not a compatibility claim for
other mounts.

## 1. Inventory the complete control and optical chain

Record the exact model shown on each item, rather than inferring a model from a
family name or an accessory:

| Part | Record |
|---|---|
| Mount head | Manufacturer, model, visible revision and serial in a private inventory; keep identifying photos private. |
| Motor drives and controller | Motor-board labels, controller model/revision, firmware shown by the handset or driver, and sockets used. Keep firmware separate from hardware revision and store identifying observations privately. |
| Handset or control app | Model/version, firmware, database text, and any displayed hardware revision. Record fields under their displayed labels in private notes. |
| Adapter or cable | Product/model and firmware, connection mode, connector, target address, and transport in private notes. Radio module certification IDs identify the radio module and may not identify the adapter. |
| Telescope and guide optics | Tube model and focal length; guide scope or off-axis guider model and focal length; note how the guide optic is attached. |
| Camera | Camera model, pixel pitch in micrometres, driver, and exact INDI device name. Leave it unconfigured until a camera and measurement source are selected. |

Use labels, manufacturer manuals, firmware information screens, and the
computer driver's device list as separate evidence sources. A manual or catalog
number is not a mount serial. If a label is absent or hard to reach, record
“not found during inspection”; do not claim that no hidden label exists.

For a SynScan V3 handset, the documented version screen is
`Utility Func > Show Information > Version`; scroll through its H.C. Firmware,
Database, H.C. Hardware, Motor Controller, and H.C. Serial # fields. The
`SETUP > Auto Guide Speed` menu shows the selected handset guide rate. These
paths are SynScan-specific; use the matching manual for other controllers.

## 2. Identify the control path and device names

Trace the physical connection from computer to mount: direct adapter, handset
or app, USB/serial cable, or another route. For the route you plan to use, copy
the transport, address, port, driver executable, and displayed device name from
its manual and driver documentation. Confirm the live device name from the
driver's property list; do not guess it from the mount model.

Keep these endpoints distinct:

- the computer's INDI server host and port;
- the mount adapter or serial endpoint;
- any app endpoint used by a separate controller app.

A successful network or serial connection does not establish GoTo, tracking, or
pulse-guide support. Verify the features exposed by the exact driver and mount
combination before relying on them. Operate one mount controller at a time.

For Wi-Fi discovery, record the SSID, computer's local IP, adapter target IP,
route interface, transport, and port in private notes when they help identify
the connection. Do not publish SSIDs, local addresses, Wi-Fi passwords, BSSIDs,
or personal network reports. Use placeholders or vendor defaults in public
examples. A local route lookup is not a mount compatibility test.

## 3. Record the guide rate without calling it calibration

Record the handset or driver guide-rate setting exactly as displayed, including
whether the RA and Dec rates are independently configurable. A selected rate
states how fast the controller intends to move during a guide pulse; it does
not measure how far this mount actually moves for a requested pulse duration.

Theoretical conversion can provide a comparison only. For a guide rate `g`
times sidereal at declination 0:

```text
ideal angular rate = 15.04157 × g  arcsec/s
ideal scale        = 15.04157 × g / 1000  arcsec/ms
```

Measure actual pulse response before using an `arcsec/ms` profile value. RA
image-plane scale also depends on declination and the camera-to-mount axis
transform.

## 4. Check camera and guide-optic scale

For pixel pitch `p` in micrometres and guide-optic focal length `f` in
millimetres, estimate the image scale as:

```text
arcsec/pixel = 206.265 × p / f
```

Use the focal length of the optic carrying the guide camera, not the main
telescope's focal length unless the camera is on the main optical path. For
non-square pixels, calculate each image axis separately. This is pixel scale;
it does not by itself convert camera X/Y into signed RA/Dec errors. That
conversion needs a reference centroid and a measured camera-to-mount rotation
and sign.

An ordinary camera driver supplies images or pixel positions. This project
needs a centroid bridge that publishes its configured
`GUIDE_ERROR_ARCSEC` contract with signed sky-axis RA/Dec errors and valid star
flux plus a strictly increasing `CAPTURE_UNIX_SECONDS` timestamp for the
source capture. The timestamp must use Unix seconds and the bridge clock must
be synchronized with the guider host; repeating a cached sample does not renew
its age. A camera model or device name alone does not make guiding ready. With
no camera, use offline profile inspection and hardware discovery;
pulse-response calibration and star-based periodic-error measurement must wait.

## 5. Measure pulse response and backlash

Only after the mount path, guide rate, centroid scale, and axis orientation are
known, measure pulse response with tracking on and closed-loop corrections off.
Use short pulses that are appropriate for the verified mount and record each
axis separately:

1. Record a no-pulse centroid baseline and its time span.
2. Apply a short RA pulse in each direction and a short Dec pulse in each
   direction; repeat at several durations.
3. Convert centroid displacement to sky-axis arcseconds, subtract baseline
   drift, and fit displacement against requested milliseconds for each axis
   and direction.
4. Record guide rate, declination, duration, displacement, sign, repeatability,
   and any backlash or stiction before updating the profile.

The fitted displacement slopes give empirical `arcsec_per_ms` values for that
setup and observing geometry. Do not derive them from the handset guide rate
alone. If the axis direction or response is unclear, stop and inspect the
driver's pulse properties before continuing.

## 6. Measure raw periodic error separately

To estimate a mount's worm period and raw RA periodic error, collect
time-stamped star-centroid measurements while sidereal tracking is on and mount
guide output is disabled. Capture several complete cycles, then inspect the
raw log, detrending, repeatability, and uncertainty. A reported PEC-training
interval, gear-family estimate, or actively guided trace is not a measurement
of the physical period on this mount. Determine wheel tooth count from a
reliable specification or direct documented evidence; do not infer it by
decoding an unverified protocol response.

The capture's time origin is not a physical worm-phase reference. Keep PE
playback disabled until the application can synchronize a measured curve to a
reliable physical phase signal. A correct-looking period alone is not enough to
arm feed-forward correction.

## 7. Write the profile with evidence attached

Copy `config/default.toml` to `config/profiles/<equipment-name>.toml`. Keep a
camera blank unless a physical camera and the required measurement source are
identified. Keep active aggressiveness at zero until pulse response has been
measured; keep PE playback disabled until its phase can be synchronized.

In comments next to each equipment-specific value, label its basis:

- **owner-observed** — directly reported or read from the actual setup; keep
  the observation private unless its publication is explicitly authorized;
- **manufacturer/manual** — published for the exact identified model;
- **measured** — collected from the actual equipment, with date and method;
- **calculated** — show the formula and its input assumptions;
- **inferred** — explain the evidence and remaining uncertainty;
- **unknown/unverified** — leave unset where possible and do not fill by analogy.

Preserve raw displayed text and raw protocol replies in private notes when
they matter. Publish them only with explicit authorization. Record inquiry,
transport, date, and source for private analysis, but do not assign units or
subsystem meaning to an undecoded value. Keep firmware version, hardware
revision, guide rate, measured pulse response, worm period, and worm-wheel
tooth count as separate facts. Use `inspect` to review the TOML profile offline
before any hardware connection.
