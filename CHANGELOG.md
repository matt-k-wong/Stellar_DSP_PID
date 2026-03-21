# Changelog

All notable changes to Stellar DSP PID will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- Initial ZDF SVF filter implementation (Zavalishin topology)
- Wavetable PE correction engine with Hermite interpolation
- ANC engine combining feed-forward + ZDF-filtered PID
- Async INDI TCP client
- Tokio-based orchestration loop
- Calibration mode for PE curve acquisition
- Criterion benchmarks for DSP hot path

## [0.1.0] - 2024-01-01
### Added
- Project scaffolding
