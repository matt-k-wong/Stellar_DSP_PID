# Agent guidance

This repository controls telescope mounts through INDI. Prioritize offline
inspection and evidence-based configuration over live hardware experiments.
Read this file before changing code, profiles, or setup documentation.

## Safety and scope

- Inspect `git status` first and preserve existing user changes. Keep edits
  limited to the requested files and do not commit unless the user asks.
- Do not connect to a live mount, Wi-Fi adapter, or INDI server; run a network
  probe; or send mount commands unless the user explicitly requests that action.
  When the task is documentation or code work, never execute the probe against
  hardware.
- Never issue unrequested slew, guide, initialization, tracking, configuration,
  PEC-training, or other motion/state-changing commands. For authorized
  discovery, use only documented read-only inquiries and keep timeouts bounded.
- Do not run or add tests unless the user explicitly asks for testing. If asked,
  run only the relevant checks and do not weaken assertions to obtain a pass.
- Do not publish private owner-specific equipment identifiers, firmware
  versions, SSIDs, local addresses, serials, photo filenames, raw replies, or
  personal reports. Keep actual observations in private notes unless the user
  explicitly authorizes publication of those specific facts. Prefer clearly
  illustrative or synthetic examples in public profiles and documentation.

## Profiles and evidence

- Keep `config/default.toml` generic and equipment-neutral. Put gear-specific
  values in a named profile under `config/profiles/`; do not add presumed
  cameras or adapters to the generic configuration.
- Preserve unknowns. Do not infer compatibility, motor-controller hardware
  revision, worm-wheel tooth count, physical worm period, or guide-pulse
  response from a family name, manual/catalog number, selected guide rate, or
  an unrelated model.
- Keep handset firmware, controller firmware, database text, and hardware
  revision as distinct fields. Keep a configured guide rate distinct from
  measured `arcsec/ms` pulse response. A PEC training interval is not a measured
  worm period.
- Label public profile values in nearby comments as manufacturer/manual-listed,
  measured from public/authorized evidence, calculated, inferred, or illustrative.
  Do not publish actual private owner observations without explicit
  authorization for those facts. When authorized measurements are published,
  record date, method, conditions, and source. Never publish private display
  text or raw protocol replies by default.
- Do not assign units or subsystem meaning to a raw reply without documented
  framing and decoding. Do not treat an adapter response as proof that guiding
  works on the complete mount/controller combination.
- Keep guide mode closed unless the configured measurement device publishes
  verified signed RA/Dec errors in sky arcseconds. Pixel coordinates need a
  reference centroid, image scale, and measured camera-to-mount transform.
- Keep active feedback disarmed until pulse response is measured. Keep PE
  playback disabled until a measured curve is synchronized to a reliable
  physical worm-phase reference.

## Research and documentation

- Prefer primary manuals, manufacturer material, and the exact driver's
  documentation or source. Link the source and distinguish its claims from
  public or authorized observations, calculations, and inferences.
- Use [the equipment discovery guide](docs/gear_profile_discovery.md) for
  reusable profile workflow. Keep private owner-specific findings in private
  notes. Public examples must be generic or clearly illustrative, never a
  disguised inventory of someone's equipment.
- Keep instructions actionable and model-specific only where evidence supports
  the exact model. Say when a transport or feature remains unverified.
