# 🎹 Terminal Piano (Rust & Ratatui)

A polyphonic, low-latency terminal piano synthesizer written in **Rust** using **Ratatui** and **Rodio**. Features realistic acoustic decay modeling and physical keyboard sustain pedal support.

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-2024%20edition-orange.svg)

---

## ✨ Features

- **Zero Audio Assets Required**: Pure real-time mathematical waveform synthesis (fundamental sine + acoustic harmonics).
- **Acoustic String & Decay Dynamics**:
  - Minimum ringing duration modeling (mimics the kinetic energy of hammer strikes on strings).
  - Smooth damper release curve (prevents pops and clicks).
  - Dynamic ringing duration control via arrow keys.
- **Sustain Pedal**: Hold `Space` to keep notes ringing for ~4.5 seconds with acoustic resonance.
- **Modern Grid TUI**: Pixel-perfect layout powered by `ratatui` with responsive rounded keys and real-time active key highlights.
- **Hardware-Level Key Release**: Integrated with the Kitty Keyboard Protocol to detect physical key release events instantly on modern terminals.

---

## 🎮 Controls

### 🎼 Basic Keys (Middle Octave)
| Key | Note | Frequency |
|:---:|:---:|:---:|
| `n` | **Do (C4)** | 261.6 Hz |
| `j` | **Re (D4)** | 293.7 Hz |
| `ı` / `i` | **Mi (E4)** | 329.6 Hz |
| `m` | **Fa (F4)** | 349.2 Hz |
| `k` | **Sol (G4)** | 392.0 Hz |
| `o` | **La (A4)** | 440.0 Hz |
| `p` | **Si (B4)** | 493.9 Hz |

### 🎛️ Modifiers & Controls
- **`Space`**: Sustain Pedal (Hold down to sustain active notes)
- **`←` / `→`**: Decrease / Increase minimum ringing duration (100 ms – 2000 ms)
- **`q`** or **`Esc`**: Quit application

---

## 🚀 Installation & Running

### Prerequisites
- [Rust & Cargo](https://rustup.rs/) (1.80+)
- ALSA / PipeWire development libraries:
  ```bash
  # Arch Linux
  sudo pacman -S alsa-lib pipewire
  # Debian/Ubuntu
  sudo apt-get install libasound2-dev
  # Fedora
  sudo dnf install alsa-lib-devel
  ```

### Build and Run
```bash
git clone https://github.com/jeftbt/terminal-piano.git
cd terminal-piano
cargo run --release
```

---

## 📜 License
MIT License. Created by [jeftbt](https://github.com/jeftbt).
