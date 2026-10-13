use std::fs::File;
use std::io::stdout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

/// RustySynth SoundFont tabanlı gerçek zamanlı stereo ses kaynağı
struct SoundFontSource {
    synth: Arc<Mutex<Synthesizer>>,
    sample_rate: u32,
    buffer_left: Vec<f32>,
    buffer_right: Vec<f32>,
    buffer_pos: usize,
}

impl SoundFontSource {
    fn new(synth: Arc<Mutex<Synthesizer>>, sample_rate: u32) -> Self {
        Self {
            synth,
            sample_rate,
            buffer_left: vec![0.0; 128],
            buffer_right: vec![0.0; 128],
            buffer_pos: 256, // başlangıçta boş kabul et
        }
    }
}

impl Iterator for SoundFontSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        // İki kanallı interleave (L, R, L, R...) akış
        if self.buffer_pos >= self.buffer_left.len() * 2 {
            let mut synth = self.synth.lock().unwrap();
            synth.render(&mut self.buffer_left[..], &mut self.buffer_right[..]);
            self.buffer_pos = 0;
        }

        let frame_index = self.buffer_pos / 2;
        let is_left = (self.buffer_pos % 2) == 0;
        self.buffer_pos += 1;

        let sample = if is_left {
            self.buffer_left[frame_index]
        } else {
            self.buffer_right[frame_index]
        };

        Some(sample)
    }
}

impl Source for SoundFontSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        2 // Gerçek Stereo
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

const NOTE_NAMES: [&str; 12] = [
    "Do", "Do#", "Re", "Re#", "Mi", "Fa", "Fa#", "Sol", "Sol#", "La", "La#", "Si",
];

// Semitone (0..11) ve Oktavdan MIDI nota numarası:
// C4 (Orta Do) = 60
fn to_midi_note(semitone: u8, octave: i8) -> i32 {
    ((octave + 1) * 12 + semitone as i8) as i32
}

#[derive(Clone, Copy)]
struct KeySlot {
    semitone: u8, // 0..11
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

fn map_key_to_slot(code: KeyCode, modifiers: KeyModifiers) -> Option<(usize, i8)> {
    let has_shift = modifiers.contains(KeyModifiers::SHIFT);
    let has_alt = modifiers.contains(KeyModifiers::ALT);

    let c = match code {
        KeyCode::Char(ch) => ch,
        _ => return None,
    };

    let (slot_idx, is_upper) = match c {
        // Alt sıra
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

    let octave = if is_upper {
        if has_alt {
            6
        } else if has_shift || c.is_uppercase() {
            5
        } else {
            4
        }
    } else {
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
    last_played_info: &Option<(String, i8, i32)>,
    pedal_pressed: bool,
) {
    let size = frame.area();

    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Başlık
            Constraint::Length(7),  // Üst Sıra Piyano (Tizler)
            Constraint::Length(7),  // Alt Sıra Piyano (Baslar)
            Constraint::Length(3),  // Durum Çubuğu
            Constraint::Length(2),  // Kısayollar
        ])
        .margin(1)
        .split(size);

    // 1. Başlık
    let header = Paragraph::new(Line::from(vec![
        Span::styled("🎹 ", Style::default()),
        Span::styled(
            "ACOUSTIC GRAND PIANO (SoundFont Engine)",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" — Gerçek Örneklenmiş Steinway Akustik Sesler", Style::default().fg(Color::DarkGray)),
    ]))
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(header, main_layout[0]);

    // Piyano Tuş Çizimi
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
                (Color::DarkGray, Color::DarkGray, Color::White)
            } else {
                (Color::White, Color::Reset, Color::White)
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

    // 2. Üst Sıra
    render_octave_row(
        frame,
        main_layout[1],
        " 🎼 TİZ GRUP: [Normal: Oktav 4 (Merkez)] │ [Shift: Oktav 5 (+1)] │ [Alt: Oktav 6 (+2)] ",
        &SLOTS[12..24],
        &active_keys[12..24],
    );

    // 3. Alt Sıra
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
            Constraint::Percentage(70),
            Constraint::Percentage(30),
        ])
        .split(main_layout[3]);

    let status_text = if let Some((name, oct, midi)) = last_played_info {
        Line::from(vec![
            Span::styled(" ▶ Çalınan Nota: ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{}{} ", name, oct), Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD)),
            Span::styled(format!("(MIDI Note #{})", midi), Style::default().fg(Color::DarkGray)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" Durum: ", Style::default().fg(Color::DarkGray)),
            Span::styled("Gerçek Akustik Piyano hazır. Tuşlara basarak çalabilirsiniz.", Style::default().fg(Color::DarkGray)),
        ])
    };

    let status_box = Paragraph::new(status_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(if last_played_info.is_some() { Color::Green } else { Color::DarkGray })),
    );
    frame.render_widget(status_box, status_cols[0]);

    // Pedal Kutusu
    let (pedal_text, pedal_bg, pedal_fg, pedal_border) = if pedal_pressed {
        (
            " PEDAL: AKTİF (SUSTAIN) ",
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
    frame.render_widget(pedal_box, status_cols[1]);

    // 5. Kısayol Bilgisi
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Pedal: ", Style::default().fg(Color::DarkGray)),
        Span::styled("SPACE (Basılı Tut)", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        Span::styled("  │  Tizler: ", Style::default().fg(Color::DarkGray)),
        Span::styled("Shift (+1) / Alt (+2)", Style::default().fg(Color::Yellow)),
        Span::styled("  │  Baslar: ", Style::default().fg(Color::DarkGray)),
        Span::styled("Shift (-1) / Alt (-2)", Style::default().fg(Color::Yellow)),
        Span::styled("  │  Çıkış: ", Style::default().fg(Color::DarkGray)),
        Span::styled("ESC", Style::default().fg(Color::Red)),
    ]))
    .alignment(Alignment::Center);
    frame.render_widget(footer, main_layout[4]);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. SoundFont dosyasını yükle
    let sf2_path = "assets/piano.sf2";
    let mut sf2_file = File::open(sf2_path)
        .map_err(|e| format!("SoundFont dosyası açılamadı ({}): {}", sf2_path, e))?;
    let sound_font = Arc::new(SoundFont::new(&mut sf2_file)?);

    // 2. Synthesizer motorunu 44100 Hz stereo olarak başlat
    let sample_rate = 44100;
    let settings = SynthesizerSettings::new(sample_rate as i32);
    let synth = Arc::new(Mutex::new(Synthesizer::new(&sound_font, &settings)?));

    // 3. Rodio ses akışına bağla
    let (_stream, stream_handle) = OutputStream::try_default()?;
    let sink = Sink::try_new(&stream_handle)?;
    let source = SoundFontSource::new(Arc::clone(&synth), sample_rate);
    sink.append(source);
    sink.play();

    // 4. Terminal TUI Kurulumu
    enable_raw_mode()?;
    let mut stdout = stdout();
    stdout.execute(EnterAlternateScreen)?;

    let _ = stdout.execute(PushKeyboardEnhancementFlags(
        KeyboardEnhancementFlags::REPORT_EVENT_TYPES,
    ));

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Aktif tuşlar ve nota takibi: (Slot bazında o an basılan MIDI notası)
    let mut active_midi_notes: [Option<i32>; 24] = [None; 24];
    let mut last_press_time: [Option<Instant>; 24] = Default::default();
    let mut received_real_release = false;

    let pedal_atomic = Arc::new(AtomicBool::new(false));
    let mut last_space_time: Option<Instant> = None;
    let fallback_release_timeout = Duration::from_millis(750);

    let mut is_active_array = [false; 24];
    let mut last_played_info: Option<(String, i8, i32)> = None;

    loop {
        let pedal_held = pedal_atomic.load(Ordering::Relaxed);
        terminal.draw(|f| draw_ui(f, &is_active_array, &last_played_info, pedal_held))?;

        let mut state_changed = false;

        if event::poll(Duration::from_millis(15))? {
            let ev = event::read()?;
            if let Event::Key(key_event) = ev {
                if key_event.code == KeyCode::Esc {
                    break;
                }

                // SPACE -> Sustain Pedalı (MIDI Controller #64)
                if key_event.code == KeyCode::Char(' ') {
                    if key_event.kind == KeyEventKind::Press || key_event.kind == KeyEventKind::Repeat {
                        last_space_time = Some(Instant::now());
                        if !pedal_atomic.load(Ordering::Relaxed) {
                            pedal_atomic.store(true, Ordering::Relaxed);
                            let mut s = synth.lock().unwrap();
                            s.process_midi_message(0, 0xB0, 64, 127); // Hold Pedal ON
                            state_changed = true;
                        }
                    } else if key_event.kind == KeyEventKind::Release {
                        received_real_release = true;
                        pedal_atomic.store(false, Ordering::Relaxed);
                        let mut s = synth.lock().unwrap();
                        s.process_midi_message(0, 0xB0, 64, 0); // Hold Pedal OFF
                        last_space_time = None;
                        state_changed = true;
                    }
                }

                // Kromatik Tuşlar
                if let Some((slot_idx, octave)) = map_key_to_slot(key_event.code, key_event.modifiers) {
                    if key_event.kind == KeyEventKind::Press || key_event.kind == KeyEventKind::Repeat {
                        last_press_time[slot_idx] = Some(Instant::now());

                        if active_midi_notes[slot_idx].is_none() {
                            let slot = SLOTS[slot_idx];
                            let midi_note = to_midi_note(slot.semitone, octave);

                            let mut s = synth.lock().unwrap();
                            // Velocity 95: Doğal, dolgun konser piyanisti tuşesi
                            s.note_on(0, midi_note, 95);

                            active_midi_notes[slot_idx] = Some(midi_note);
                            last_played_info = Some((
                                NOTE_NAMES[slot.semitone as usize].to_string(),
                                octave,
                                midi_note,
                            ));
                            state_changed = true;
                        }
                    } else if key_event.kind == KeyEventKind::Release {
                        received_real_release = true;
                        if let Some(midi_note) = active_midi_notes[slot_idx].take() {
                            let mut s = synth.lock().unwrap();
                            s.note_off(0, midi_note);
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
                    let mut s = synth.lock().unwrap();
                    s.process_midi_message(0, 0xB0, 64, 0);
                    last_space_time = None;
                    state_changed = true;
                }
            }
        }

        // Tuşlar Fallback Timeout
        if !received_real_release {
            let now = Instant::now();
            for i in 0..24 {
                if let Some(last_time) = last_press_time[i] {
                    if now.duration_since(last_time) > fallback_release_timeout {
                        if let Some(midi_note) = active_midi_notes[i].take() {
                            let mut s = synth.lock().unwrap();
                            s.note_off(0, midi_note);
                        }
                        last_press_time[i] = None;
                        state_changed = true;
                    }
                }
            }
        }

        if state_changed {
            for i in 0..24 {
                is_active_array[i] = active_midi_notes[i].is_some();
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
