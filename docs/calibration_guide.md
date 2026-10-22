# Calibration and tuning notes

Calibration requires real guide-star measurements. With no guide camera, use
the offline `inspect` command and hardware checklist in
[the SkyView first setup guide](skyview_pro_example_setup.md). Do not run PE
capture or guide mode with assumed camera units.

## 1. Establish the measurement units

The current guider accepts only a configured measurement device publishing the
project's `GUIDE_ERROR_ARCSEC` property. `indi.camera_device` names that INDI
source and may be the future centroid bridge rather than the physical camera.
Ordinary camera drivers cannot publish the custom property by themselves. Its
RA and Dec values must be signed
sky-axis errors in arcseconds, `STAR_FLUX` must be positive, and
`CAPTURE_UNIX_SECONDS` must carry the source capture time in Unix seconds. Do
not substitute the INDI message arrival time. Timestamps must be finite,
strictly increasing, and synchronized with the guider host's clock. Raw camera
pixels need a reference centroid, image scale, and camera-to-mount axis
transform first. See [INDI setup](indi_setup.md) for the property contract.

Guide mode rejects future, repeated, out-of-order, and stale captures. A capture
older than `max(3 × guide_exposure, 5 seconds)` is stale; replaying a cached
measurement does not renew freshness. The last accepted capture age is checked
again before every pulse.

For a camera pixel pitch `p` in micrometres and guide optic focal length `f` in
millimetres, the image scale is:

```text
scale = 206.265 × p / f  arcsec/pixel
```

This converts pixel motion to an angle, but not to RA/Dec. Measure the camera
axis orientation and signs with short pulses after the mount connection and
guide-rate setting are verified.

## 2. Measure pulse response before tuning feedback

At Dec near 0, with sidereal tracking and the guide rate recorded, test short
RA and Dec pulses in both directions. Begin at conservative durations. Measure
the centroid displacement after each pulse, convert pixels to arcseconds, and
subtract baseline drift. Fit displacement versus commanded milliseconds
separately for RA+, RA−, Dec+, and Dec−. The fitted slopes, plus direction/sign
handling, determine the effective `arcsec_per_ms_ra` and
`arcsec_per_ms_dec` values.

For reference only, a 0.5× sidereal guide rate corresponds ideally to
`15.04157 × 0.5 / 1000 = 0.007521 arcsec/ms` at Dec 0. This is a theoretical
starting estimate, not a mount measurement. RA image-plane scale changes with
`cos(Dec)`; measure again at the intended observing declination when practical.

## 3. Record raw periodic error

Raw PE must be captured with mount tracking on and guide output disabled. Use a
guide star near the celestial equator, retain exposure timestamps, and record
at least 55–60 minutes (six cycles at 598.36 s or seven at 480 s).
Do not combine a guided residual with raw PE. Compare cycle-to-cycle phase and
amplitude, and retain the 480 s and roughly 598–600 s candidates until this
mount's data resolves the disagreement.

The SkyView profile's 598.36 s is the calculation for a 144-tooth wheel, not a
measured value. The Orion manual's 480 s PEC training interval conflicts with
that calculation but does not itself establish the physical worm period. The
exact wheel and controller revision still need identification.

## 4. Curve capture is not yet PE playback

The CLI can record values from verified `GUIDE_ERROR_ARCSEC` measurements:

```sh
cargo run -- --config path/to/profile.toml calibrate \
  --cycles 6 --output config/pe_curve.json
```

It refuses to run without a configured measurement source, nonzero worm period,
and at least one verified measurement. Six cycles are about 61 minutes at the
SkyView profile's provisional 598.36-second period; use seven cycles when
evaluating the 480-second candidate. The resulting JSON is an analysis artifact,
not a replay-ready mount correction. Its phase origin comes from capture
timestamps, while the mount's physical worm phase is not read by this app.

PE playback remains blocked in guide mode until a trustworthy physical worm
phase reference is available. Power loss, changing the worm period, or losing
phase requires synchronization again. A curve with an arbitrary phase can
increase periodic error, so do not bypass that gate by setting
`enable_pe_correction = true`.

## 5. Tune feedback only after units are verified

Start with PE disabled. Keep the same load, balance, pier side, guide star,
declination, exposure, and cable routing when comparing captures. Record RA/Dec
RMS, pulse counts and durations, backlash, wind, and any structural flexure.
Change one setting at a time. The project has not established a performance
target or guaranteed improvement for this mount.
