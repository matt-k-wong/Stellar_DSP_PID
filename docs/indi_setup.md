# INDI Setup Guide

## Installing INDI on Raspberry Pi OS (Bookworm)

```bash
sudo apt-get update
sudo apt-get install indi-bin indi-asi libindi-dev
```

For other cameras, replace `indi-asi` with the appropriate driver package
(e.g. `indi-qhy`, `indi-playerone`, `indi-svbony`).

## Starting indiserver

Start indiserver with the relevant camera and mount drivers:

```bash
# ZWO camera + EQMod mount
indiserver indi_asi_ccd indi_eqmod

# Background service (systemd)
sudo systemctl enable --now indiserver
```

## Verifying INDI Properties

Use `indiprop` (part of INDI) to list and inspect properties:

```bash
# List all devices
indiprop -h localhost -p 7624 -g "*" 2>/dev/null | grep "^Device:"

# Dump guide star position property
indiprop -h localhost -p 7624 -g "ZWO CCD ASI120MM Mini" "RAPID_GUIDE_DATA"
```

## Finding Your Mount's INDI Name

```bash
indiprop -h localhost -p 7624 -g "*" 2>/dev/null | grep -i "telescope\|mount\|eq"
```

Common names:
- EQMod (SkyWatcher): `EQMod Mount`
- iEQ / iOptron: `iEQ Mount`
- Celestron CGX: `Celestron CGX`

Update `[indi] mount_device` in your `config/default.toml` accordingly.

## Camera Configuration

Stellar DSP PID expects the camera driver to expose guide star data under
`GUIDE_STAR_POSITION` or `RAPID_GUIDE_DATA`.  If your driver uses a different
property name, edit `src/indi_client.rs` — see the `match self.prop_name.as_str()`
block in `IndiParser::emit_event()`.

Alternatively, you can use PHD2 to perform centroiding and publish the guide
deviation to INDI via its INDI server mode.
