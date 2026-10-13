# 🎹 Terminal Piano (Rust & Ratatui)

A polyphonic terminal piano written in **Rust**. Real sampled piano sounds via a SoundFont engine (`rustysynth`), a pixel-aligned TUI (`ratatui`), and hardware key-release detection through the Kitty keyboard protocol.

![Rust](https://img.shields.io/badge/rust-2024%20edition-orange.svg)

---

## ✨ Features

- **Sampled piano sound**: General MIDI acoustic grand piano from an embedded SoundFont (`TimGM6mb.sf2`), no math synthesis.
- **2 chromatic octaves under your hands**, extended to **6 octaves** with `Shift` / `Alt`.
- **Sustain pedal** on `Space` (MIDI CC #64).
- **Minimum ring time**: a quick tap still rings for a configurable duration (`←` / `→`).
- **Polyphony**: chords and two-handed playing.
- **Single binary**: the SoundFont is compiled into the executable.

---

## 🎮 Controls

```text
Black keys (#):     [S]   [D]         [G]   [H]   [J]          [2]   [3]         [5]   [6]   [7]
White keys:      [Z]   [X]   [C]   [V]   [B]   [N]   [M]    [Q]   [W]   [E]   [R]   [T]   [Y]   [U]
                 Do    Re    Mi    Fa    Sol   La    Si     Do    Re    Mi    Fa    Sol   La    Si
                └────────────── Lower row (bass) ──────────┘ └────────────── Upper row (treble) ─────────┘
```

| Row | Normal | `Shift` | `Alt` |
|---|---|---|---|
| Upper (`Q…U`, `2…7`) | Octave 4 | Octave 5 | Octave 6 |
| Lower (`Z…M`, `S…J`) | Octave 3 | Octave 2 | Octave 1 |

| Key | Action |
|---|---|
| `Space` | Sustain pedal (hold) |
| `←` / `→` | Minimum ring time −/+ 50 ms (100–3000 ms) |
| `Esc` | Quit |

> **Tip:** Use a terminal that supports the Kitty keyboard protocol (Kitty, WezTerm, foot, Ghostty). It gives instant key-release detection and layout-independent keys (e.g. `Shift+2` works on Turkish Q).

---

## 🚀 Build & Run

Prerequisites: Rust 1.85+, ALSA dev libraries.

```bash
# Arch
sudo pacman -S alsa-lib
# Debian/Ubuntu
sudo apt-get install libasound2-dev
```

```bash
git clone https://github.com/jeftbt/terminal-piano.git
cd terminal-piano
cargo run --release
```

---

## 📜 License

Code: MIT. SoundFont: TimGM6mb by Tim Brechbill (GPL-2.0).
