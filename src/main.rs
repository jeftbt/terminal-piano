use std::f32::consts::PI;
use std::io::stdout;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Terminal;
use rodio::source::Source;
use rodio::{OutputStream, Sink};

/// Akustik piyano titreşim ve sönüm dinamiklerine sahip ses kaynağı
struct ContinuousTone {
    freq: f32,
    sample_rate: u32,
    sample_clock: usize,
    is_active: Arc<AtomicBool>,
    pedal_active: Arc<AtomicBool>,
    min_press_ms: Arc<AtomicU32>,
    release_clock: Option<usize>,
    normal_release_samples: usize,
    pedal_release_samples: usize,
    release_start_gain: f32,
    current_gain: f32,
}

impl ContinuousTone {
    fn new(
        freq: f32,
        sample_rate: u32,
        is_active: Arc<AtomicBool>,
        pedal_active: Arc<AtomicBool>,
        min_press_ms: Arc<AtomicU32>,
    ) -> Self {
        let normal_release_samples = (sample_rate as f32 * 0.45) as usize;
        let pedal_release_samples = (sample_rate as f32 * 0.70) as usize;

        Self {
            freq,
            sample_rate,
            sample_clock: 0,
            is_active,
            pedal_active,
            min_press_ms,
            release_clock: None,
            normal_release_samples,
            pedal_release_samples,
            release_start_gain: 1.0,
            current_gain: 1.0,
        }
    }
}

impl Iterator for ContinuousTone {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let key_pressed = self.is_active.load(Ordering::Relaxed);
        let pedal_held = self.pedal_active.load(Ordering::Relaxed);

        let min_ms = self.min_press_ms.load(Ordering::Relaxed);
        let min_press_samples = (self.sample_rate as f32 * (min_ms as f32 / 1000.0)) as usize;

        let min_duration_reached = self.sample_clock >= min_press_samples;

        // Release başladığı anda mevcut ses kazancını dondur
        if min_duration_reached && !key_pressed && !pedal_held && self.release_clock.is_none() {
            self.release_clock = Some(0);
            self.release_start_gain = self.current_gain;
        }

        let base_volume = 0.38;

        if let Some(rel) = self.release_clock {
            let max_release = if self.sample_clock > min_press_samples + 5000 {
                self.pedal_release_samples
            } else {
                self.normal_release_samples
            };

            if rel >= max_release {
                return None;
            }
            let progress = rel as f32 / max_release as f32;
            let decay = (1.0 - progress).powi(2);
            self.current_gain = self.release_start_gain * decay;
            self.release_clock = Some(rel + 1);
        } else if !key_pressed && pedal_held {
            let sustain_samples = (self.sample_rate as f32 * 4.5) as usize;
            if self.sample_clock > sustain_samples {
                return None;
            }
            let natural_decay = (1.0 - (self.sample_clock as f32 / sustain_samples as f32)).powf(1.3);
            self.current_gain = natural_decay;
        } else {
            let attack_samples = (self.sample_rate as f32 * 0.012) as usize;
            if self.sample_clock < attack_samples {
                self.current_gain = self.sample_clock as f32 / attack_samples as f32;
            } else {
                self.current_gain = 1.0;
            }
        }

        let amplitude = base_volume * self.current_gain;
        let t = self.sample_clock as f32 / self.sample_rate as f32;
        self.sample_clock += 1;

        let f = self.freq;
        let wave = (2.0 * PI * f * t).sin()
            + 0.28 * (4.0 * PI * f * t).sin()
            + 0.08 * (6.0 * PI * f * t).sin();

        Some(wave * amplitude)
    }
}

impl Source for ContinuousTone {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        1
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

// 12 Nota için semitone ofsetleri (C = 0, C# = 1, ... B = 11)
// C0 frekansı yaklaşık 16.3516 Hz
fn note_frequency(semitone: u8, octave: i8) -> f32 {
    let midi_note = (octave + 1) * 12 + semitone as i8;
    // A4 (MIDI 69) = 440 Hz
    440.0 * 2.0_f32.powf((midi_note as f32 - 69.0) / 12.0)
}

const NOTE_NAMES: [&str; 12] = [
    "Do", "Do#", "Re", "Re#", "Mi", "Fa", "Fa#", "Sol", "Sol#", "La", "La#", "Si",
];

/// Klavye Slotu Bilgisi
#[derive(Clone, Copy)]
struct KeySlot {
    semitone: u8,       // 0..11
    key_label: &'static str,
    is_sharp: bool,
}

// 2 Oktavlık 24 Tuşluk Kromatik Slot Haritası
const SLOTS: [KeySlot; 24] = [
    // --- ALT SIRA (Oktav 3 Bazlı) ---
    KeySlot { semitone: 0,  key_label: "Z", is_sharp: false }, // C3 (Do)
    KeySlot { semitone: 1,  key_label: "S", is_sharp: true  }, // C#3
    KeySlot { semitone: 2,  key_label: "X", is_sharp: false }, // D3 (Re)
    KeySlot { semitone: 3,  key_label: "D", is_sharp: true  }, // D#3
    KeySlot { semitone: 4,  key_label: "C", is_sharp: false }, // E3 (Mi)
    KeySlot { semitone: 5,  key_label: "V", is_sharp: false }, // F3 (Fa)
    KeySlot { semitone: 6,  key_label: "G", is_sharp: true  }, // F#3
    KeySlot { semitone: 7,  key_label: "B", is_sharp: false }, // G3 (Sol)
    KeySlot { semitone: 8,  key_label: "H", is_sharp: true  }, // G#3
    KeySlot { semitone: 9,  key_label: "N", is_sharp: false }, // A3 (La)
    KeySlot { semitone: 10, key_label: "J", is_sharp: true  }, // A#3
    KeySlot { semitone: 11, key_label: "M", is_sharp: false }, // B3 (Si)

    // --- ÜST SIRA (Oktav 4 Bazlı) ---
    KeySlot { semitone: 0,  key_label: "Q", is_sharp: false }, // C4 (Do)
    KeySlot { semitone: 1,  key_label: "2", is_sharp: true  }, // C#4
    KeySlot { semitone: 2,  key_label: "W", is_sharp: false }, // D4 (Re)
    KeySlot { semitone: 3,  key_label: "3", is_sharp: true  }, // D#4
    KeySlot { semitone: 4,  key_label: "E", is_sharp: false }, // E4 (Mi)
    KeySlot { semitone: 5,  key_label: "R", is_sharp: false }, // F4 (Fa)
    KeySlot { semitone: 6,  key_label: "5", is_sharp: true  }, // F#4
    KeySlot { semitone: 7,  key_label: "T", is_sharp: false }, // G4 (Sol)
    KeySlot { semitone: 8,  key_label: "6", is_sharp: true  }, // G#4
    KeySlot { semitone: 9,  key_label: "Y", is_sharp: false }, // A4 (La)
    KeySlot { semitone: 10, key_label: "7", is_sharp: true  }, // A#4
    KeySlot { semitone: 11, key_label: "U", is_sharp: false }, // B4 (Si)
];

/// Basılan tuşu (ve modifier'ı) slot indeksi ve efektif oktava eşler
fn map_key_to_slot(code: KeyCode, modifiers: KeyModifiers) -> Option<(usize, i8)> {
    let has_shift = modifiers.contains(KeyModifiers::SHIFT);
    let has_alt = modifiers.contains(KeyModifiers::ALT);

    // Tuş karakterini yakala
    let c = match code {
        KeyCode::Char(ch) => ch,
        _ => return None,
    };

    let (slot_idx, is_upper) = match c {
        // Alt sıra beyazlar
        'z' | 'Z' => (0, false),
        's' | 'S' => (1, false),
        'x' | 'X' => (2, false),
        'd' | 'D' => (3, false),
        'c' | 'C' => (4, false),
        'v' | 'V' => (5, false),
        'g' | 'G' => (6, false),
        'b' | 'B' => (7, false),
        'h' | 'H' => (8, false),
        'n' | 'N' => (9, false),
        'j' | 'J' => (10, false),
        'm' | 'M' => (11, false),

        // Üst sıra
        'q' | 'Q' => (12, true),
        '2' | '@' => (13, true),
        'w' | 'W' => (14, true),
        '3' | '#' => (15, true),
        'e' | 'E' => (16, true),
        'r' | 'R' => (17, true),
        '5' | '%' => (18, true),
        't' | 'T' => (19, true),
        '6' | '^' => (20, true),
        'y' | 'Y' => (21, true),
        '7' | '&' => (22, true),
        'u' | 'U' => (23, true),

        _ => return None,
    };

    // Oktav hesaplama
    let octave = if is_upper {
        // Üst sıra: Baz 4. Alt: +2 (Oktav 6), Shift: +1 (Oktav 5), Normal: Oktav 4
        if has_alt {
            6
        } else if has_shift || c.is_uppercase() {
            5
        } else {
            4
        }
    } else {
        // Alt sıra: Baz 3. Alt: -2 (Oktav 1), Shift: -1 (Oktav 2), Normal: Oktav 3
        if has_alt {
            1
        } else if has_shift || c.is_uppercase() {
            2
        } else {
            3
        }
    };

    Some((slot_idx, octave))
}

fn draw_ui(
    frame: &mut ratatui::Frame,
    active_keys: &[bool; 24],
    last_played_info: &Option<(String, f32, i8)>,
    pedal_pressed: bool,
    min_press_ms: u32,
) {
    let size = frame.area();

    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Başlık
            Constraint::Length(7),  // Üst Sıra Piyano (Tizler)
            Constraint::Length(7),  // Alt Sıra Piyano (Baslar)
            Constraint::Length(3),  // Durum Çubuğu + Kontroller
            Constraint::Length(2),  // Kısayollar
        ])
        .margin(1)
        .split(size);

    // 1. Başlık
    let header = Paragraph::new(Line::from(vec![
        Span::styled("🎹 ", Style::default()),
        Span::styled(
            "FULL CHROMATIC TERMINAL PIANO",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" — 2 Tam Oktav + Shift/Alt Modifikatörleri (Oktav 1–6)", Style::default().fg(Color::DarkGray)),
    ]))
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(header, main_layout[0]);

    // Piyano Çizim Fonksiyonu (12 Sütun)
    let render_octave_row = |f: &mut ratatui::Frame, area: ratatui::layout::Rect, title: &str, slots_slice: &[KeySlot], active_slice: &[bool]| {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(title);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let col_constraints = vec![Constraint::Ratio(1, 12); 12];
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(col_constraints)
            .split(inner);

        for (i, slot) in slots_slice.iter().enumerate() {
            let is_active = active_slice[i];
            let note_name = NOTE_NAMES[slot.semitone as usize];

            let (border_col, bg_col, fg_col) = if is_active {
                (Color::Green, Color::Green, Color::Black)
            } else if slot.is_sharp {
                (Color::DarkGray, Color::DarkGray, Color::White) // Siyah tuşlar
            } else {
                (Color::White, Color::Reset, Color::White) // Beyaz tuşlar
            };

            let lines = vec![
                Line::from(Span::styled(
                    note_name,
                    Style::default().fg(fg_col).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!("[{}]", slot.key_label),
                    Style::default().fg(if is_active { Color::Black } else { Color::Yellow }).add_modifier(Modifier::BOLD),
                )),
                Line::from(if is_active { "████" } else if slot.is_sharp { "▓▓▓▓" } else { "░░░░" }),
            ];

            let key_widget = Paragraph::new(lines)
                .alignment(Alignment::Center)
                .style(Style::default().bg(bg_col))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(border_col)),
                );
            f.render_widget(key_widget, cols[i]);
        }
    };

    // 2. Üst Sıra (Tizler: Q..U ve 2..7)
    render_octave_row(
        frame,
        main_layout[1],
        " 🎼 TİZ GRUP: [Normal: Oktav 4 (Merkez)] │ [Shift: Oktav 5 (+1)] │ [Alt: Oktav 6 (+2)] ",
        &SLOTS[12..24],
        &active_keys[12..24],
    );

    // 3. Alt Sıra (Baslar: Z..M ve S..J)
    render_octave_row(
        frame,
        main_layout[2],
        " 🎵 BAS GRUP: [Normal: Oktav 3 (Orta Bas)] │ [Shift: Oktav 2 (-1)] │ [Alt: Oktav 1 (-2)] ",
        &SLOTS[0..12],
        &active_keys[0..12],
    );

    // 4. Durum Paneli
    let status_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(main_layout[3]);

    let status_text = if let Some((name, freq, oct)) = last_played_info {
        Line::from(vec![
            Span::styled(" ▶ Son Çalınan: ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{}{} ", name, oct), Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD)),
            Span::styled(format!("({:.1} Hz)", freq), Style::default().fg(Color::DarkGray)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" Durum: ", Style::default().fg(Color::DarkGray)),
            Span::styled("Hazır. İki elle kromatik akor ve melodiler çalabilirsiniz.", Style::default().fg(Color::DarkGray)),
        ])
    };

    let status_box = Paragraph::new(status_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(if last_played_info.is_some() { Color::Green } else { Color::DarkGray })),
    );
    frame.render_widget(status_box, status_cols[0]);

    // Süre Kutusu
    let time_box = Paragraph::new(Line::from(vec![
        Span::styled(" Uzama: ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{} ms ", min_press_ms), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled("[←/→]", Style::default().fg(Color::Yellow)),
    ]))
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(time_box, status_cols[1]);

    // Pedal Kutusu
    let (pedal_text, pedal_bg, pedal_fg, pedal_border) = if pedal_pressed {
        (
            " PEDAL: AKTİF ",
            Color::Magenta,
            Color::White,
            Color::Magenta,
        )
    } else {
        (
            " PEDAL: KAPALI [SPACE] ",
            Color::Reset,
            Color::DarkGray,
            Color::DarkGray,
        )
    };

    let pedal_box = Paragraph::new(Line::from(vec![
        Span::styled(
            pedal_text,
            Style::default()
                .bg(pedal_bg)
                .fg(pedal_fg)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(pedal_border)),
    );
    frame.render_widget(pedal_box, status_cols[2]);

    // 5. Kısayol Bilgisi
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Pedal: ", Style::default().fg(Color::DarkGray)),
        Span::styled("SPACE", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        Span::styled("  │  Uzama: ", Style::default().fg(Color::DarkGray)),
        Span::styled("← / →", Style::default().fg(Color::Cyan)),
        Span::styled("  │  Tiz Oktavlar: ", Style::default().fg(Color::DarkGray)),
        Span::styled("Shift (+1) / Alt (+2)", Style::default().fg(Color::Yellow)),
        Span::styled("  │  Bas Oktavlar: ", Style::default().fg(Color::DarkGray)),
        Span::styled("Shift (-1) / Alt (-2)", Style::default().fg(Color::Yellow)),
        Span::styled("  │  Çıkış: ", Style::default().fg(Color::DarkGray)),
        Span::styled("ESC", Style::default().fg(Color::Red)),
    ]))
    .alignment(Alignment::Center);
    frame.render_widget(footer, main_layout[4]);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (_stream, stream_handle) = OutputStream::try_default()?;
    let stream_handle = Arc::new(stream_handle);

    enable_raw_mode()?;
    let mut stdout = stdout();
    stdout.execute(EnterAlternateScreen)?;

    let _ = stdout.execute(PushKeyboardEnhancementFlags(
        KeyboardEnhancementFlags::REPORT_EVENT_TYPES,
    ));

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // 24 slot için aktiflik bayrakları ve zaman takibi
    let mut active_flags: [Option<Arc<AtomicBool>>; 24] = Default::default();
    let mut last_press_time: [Option<Instant>; 24] = Default::default();
    let mut received_real_release = false;

    let pedal_atomic = Arc::new(AtomicBool::new(false));
    let mut last_space_time: Option<Instant> = None;

    let min_press_atomic = Arc::new(AtomicU32::new(500));
    let fallback_release_timeout = Duration::from_millis(750);
    let mut is_active_array = [false; 24];
    let mut last_played_info: Option<(String, f32, i8)> = None;

    loop {
        let pedal_held = pedal_atomic.load(Ordering::Relaxed);
        let current_min_ms = min_press_atomic.load(Ordering::Relaxed);
        terminal.draw(|f| draw_ui(f, &is_active_array, &last_played_info, pedal_held, current_min_ms))?;

        let mut state_changed = false;

        if event::poll(Duration::from_millis(15))? {
            let ev = event::read()?;
            if let Event::Key(key_event) = ev {
                // Çıkış (Esc tuşuyla çıkış)
                if key_event.code == KeyCode::Esc {
                    break;
                }

                // Sol ve Sağ Ok Tuşları: Uzama Süresi
                if key_event.kind == KeyEventKind::Press || key_event.kind == KeyEventKind::Repeat {
                    match key_event.code {
                        KeyCode::Left => {
                            let current = min_press_atomic.load(Ordering::Relaxed);
                            if current > 100 {
                                min_press_atomic.store(current - 50, Ordering::Relaxed);
                                state_changed = true;
                            }
                        }
                        KeyCode::Right => {
                            let current = min_press_atomic.load(Ordering::Relaxed);
                            if current < 2000 {
                                min_press_atomic.store(current + 50, Ordering::Relaxed);
                                state_changed = true;
                            }
                        }
                        _ => {}
                    }
                }

                // SPACE -> Sustain Pedalı
                if key_event.code == KeyCode::Char(' ') {
                    if key_event.kind == KeyEventKind::Press || key_event.kind == KeyEventKind::Repeat {
                        last_space_time = Some(Instant::now());
                        if !pedal_atomic.load(Ordering::Relaxed) {
                            pedal_atomic.store(true, Ordering::Relaxed);
                            state_changed = true;
                        }
                    } else if key_event.kind == KeyEventKind::Release {
                        received_real_release = true;
                        pedal_atomic.store(false, Ordering::Relaxed);
                        last_space_time = None;
                        state_changed = true;
                    }
                }

                // Kromatik Slot Kontrolü (24 Tuş + Shift / Alt)
                if let Some((slot_idx, octave)) = map_key_to_slot(key_event.code, key_event.modifiers) {
                    if key_event.kind == KeyEventKind::Press || key_event.kind == KeyEventKind::Repeat {
                        last_press_time[slot_idx] = Some(Instant::now());

                        if active_flags[slot_idx].is_none() {
                            let slot = SLOTS[slot_idx];
                            let freq = note_frequency(slot.semitone, octave);

                            let flag = Arc::new(AtomicBool::new(true));
                            let source = ContinuousTone::new(
                                freq,
                                44100,
                                Arc::clone(&flag),
                                Arc::clone(&pedal_atomic),
                                Arc::clone(&min_press_atomic),
                            );

                            if let Ok(sink) = Sink::try_new(&stream_handle) {
                                sink.append(source);
                                sink.detach();
                                active_flags[slot_idx] = Some(flag);
                                last_played_info = Some((
                                    NOTE_NAMES[slot.semitone as usize].to_string(),
                                    freq,
                                    octave,
                                ));
                                state_changed = true;
                            }
                        }
                    } else if key_event.kind == KeyEventKind::Release {
                        received_real_release = true;
                        if let Some(flag) = active_flags[slot_idx].take() {
                            flag.store(false, Ordering::Relaxed);
                            last_press_time[slot_idx] = None;
                            state_changed = true;
                        }
                    }
                }
            }
        }

        // Space Fallback Timeout
        if !received_real_release && pedal_atomic.load(Ordering::Relaxed) {
            if let Some(space_time) = last_space_time {
                if Instant::now().duration_since(space_time) > fallback_release_timeout {
                    pedal_atomic.store(false, Ordering::Relaxed);
                    last_space_time = None;
                    state_changed = true;
                }
            }
        }

        // Slotlar Fallback Timeout
        if !received_real_release {
            let now = Instant::now();
            for i in 0..24 {
                if let Some(last_time) = last_press_time[i] {
                    if now.duration_since(last_time) > fallback_release_timeout {
                        if let Some(flag) = active_flags[i].take() {
                            flag.store(false, Ordering::Relaxed);
                        }
                        last_press_time[i] = None;
                        state_changed = true;
                    }
                }
            }
        }

        if state_changed {
            for i in 0..24 {
                is_active_array[i] = active_flags[i].is_some();
            }
        }
    }

    // Temiz çıkış
    let stdout = terminal.backend_mut();
    let _ = stdout.execute(PopKeyboardEnhancementFlags);
    disable_raw_mode()?;
    stdout.execute(LeaveAlternateScreen)?;

    Ok(())
}
