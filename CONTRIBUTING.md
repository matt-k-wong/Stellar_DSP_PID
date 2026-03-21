# Contributing to Stellar DSP PID

## Development Setup

```bash
rustup toolchain install stable
rustup component add clippy rustfmt
cargo build
cargo test
cargo bench
```

## Code Standards

- All public items must have doc-comments with units stated explicitly.
- DSP hot-path functions (`#[inline(always)]`) must have zero heap allocations.
  Verify with `cargo build --release && objdump -d` or DHAT.
- Filter coefficients must cite the equation number from the reference paper.
- New DSP primitives require a unit test that verifies the −3 dB point.

## Pull Request Process

1. `cargo fmt --all`
2. `cargo clippy -- -D warnings`
3. `cargo test --all`
4. Update `CHANGELOG.md` under `[Unreleased]`

## DSP Mathematics

Before implementing a new filter topology, read:
- Zavalishin, "The Art of VA Filter Design" (free PDF, Native Instruments)
- Zölzer, "DAFX: Digital Audio Effects", Ch. 2

All filter implementations must pass a frequency-response unit test that
samples at least 5 points on the Bode magnitude plot and verifies them
within ±0.5 dB of theory.
