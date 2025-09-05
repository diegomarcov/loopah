# Loopah

A small cross-platform audio practice app focused on **looping** sections and **slowing down while preserving pitch**.

## Status

Early MVP. Current goals:

- Open an audio file (m4a/aac, mp3, wav; extensible).
- Show a waveform preview.
- Set A/B loop points and play a loop.
- Adjust speed (pitch-preserving).

## Tech

- UI: `egui`/`eframe`
- Decode: `symphonia`
- Audio I/O: `cpal`
- Time-stretch (pitch-preserving): `ssstretch` (Signalsmith Stretch, MIT)

## Build & run

```bash
rustup update
cargo run
```
