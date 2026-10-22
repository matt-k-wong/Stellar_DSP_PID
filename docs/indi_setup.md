# INDI setup and measurement input

## Camera-free first step

The `inspect` command reads a TOML profile, prints its assumptions, and exits
without contacting INDI or sending mount commands:

```sh
cargo run -- --config config/profiles/skyview-pro-example.toml inspect
```

It works before a guide camera or INDI server is installed. Use it to review the
candidate mount driver and provisional values. It does not identify hardware or
verify a connection.

## Candidate mount connection for the illustrative SkyView Pro profile

The profile's `EQMod Mount` name is a candidate. [INDI's EQMod-derived driver
documentation](https://drivers.indilib.org/mounts/sky-watcher/eqmod/eqmod/)
describes **Network/Ethernet** transport to a SynScan Wi-Fi adapter at UDP port
**11880** for the AZ-GTi. Its hotspot default is `192.168.4.1`; station mode
uses the address assigned by the local network.
The INDI server itself is a separate host and usually listens on TCP port 7624.

This path has not been confirmed end-to-end on a SkyView Pro + SynScan V3
combination. A [user report](https://groups.google.com/g/open-phd-guiding/c/gf3lauBcb7g)
describes EQMod communication over the adapter on a Raspberry Pi, but does not
confirm guide pulses on this mount. `indi_synscan_telescope` connected
to the SynScan app is a different path (TCP 11882) and is not known to provide
the pulse-guide interface this project needs.

The Wi-Fi adapter occupies the mount's hand-controller port. Do not operate the
SynScan app/handset and INDI as simultaneous mount controllers. During first
connection, verify mount type, location, time, home position, and axis
directions. Do not issue GoTo or guide pulses until those values are checked.

## Guide measurement contract

The control loop accepts only updates from the configured measurement device
named `GUIDE_ERROR_ARCSEC`, with number members:

```text
RA_ERROR_ARCSEC
DEC_ERROR_ARCSEC
STAR_FLUX
CAPTURE_UNIX_SECONDS
```

`indi.camera_device` names the INDI device that publishes this property. It is
the future centroid bridge device if separate from the physical camera. The
RA and Dec values must already be signed sky-axis errors in arcseconds,
`STAR_FLUX` must be positive, and `CAPTURE_UNIX_SECONDS` must be the source
capture time in Unix seconds, not the INDI property arrival time. Use a
sub-second timestamp with enough precision to increase on every new frame, and
synchronize the bridge clock with the host running this program.
`GUIDE_STAR_POSITION` and `RAPID_GUIDE_DATA` are intentionally ignored because
their X/Y coordinates do not include a reference centroid, pixel scale, or
camera-to-mount axis transform. A raw pixel position must never be labeled as
an arcsecond error.

Ordinary camera drivers expose images or pixel positions, not this
project-specific property. A separate centroiding bridge must select a star,
subtract a reference position, convert pixels to arcseconds, apply the measured
RA/Dec axis transform and signs, and publish the contract above. The bridge is
not included in this repository yet.
Without that verified property, the guider waits without sending corrections.
With no camera configured, guide and capture commands stop before connecting.
Capture times must be strictly increasing and no later than the host's current
time. The guider rejects a future, repeated, out-of-order, or stale frame and
pauses corrections. A capture older than `max(3 × guide_exposure, 5 seconds)`
is stale; republishing a cached frame does not refresh its age. The last
accepted capture's age is checked again before each guide pulse.

## Safe property discovery

After physical identification, an INDI setup can be explored with the INDI
tools or a client such as KStars/Ekos. Begin with one mount driver only and
inspect its advertised device and property names. Query properties; do not send
`TELESCOPE_TIMED_GUIDE_*` values during discovery. The `inspect` command remains
offline and should be used whenever the goal is only to review a profile.

See [the first setup guide](skyview_pro_example_setup.md) for a generic
hardware checklist and the distinction between adapter, app, and INDI ports.
