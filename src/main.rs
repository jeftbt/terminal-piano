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

        // Üst sıra (siyah tuşlarda Türkçe Q Shift karakterleri de yedek olarak eşlenir:
        // Shift+2 = ' , Shift+3 = ^ , Shift+5 = % , Shift+6 = & , Shift+7 = / )
        'q' | 'Q' => (12, true),
        '2' | '\'' => (13, true),
        'w' | 'W' => (14, true),
        '3' | '^' => (15, true),
        'e' | 'E' => (16, true),
        'r' | 'R' => (17, true),
        '5' | '%' => (18, true),
        't' | 'T' => (19, true),
        '6' | '&' => (20, true),
        'y' | 'Y' => (21, true),
        '7' | '/' => (22, true),
        'u' | 'U' => (23, true),

        _ => return None,
    };

    // Shift algılama: modifier, büyük harf veya Türkçe Q'daki Shift'li sembol
    let shifted = has_shift || c.is_uppercase() || matches!(c, '\'' | '^' | '%' | '&' | '/');

    let octave = if is_upper {
        if has_alt {
            6
        } else if shifted {
            5
        } else {
            4
        }
    } else {
        if has_alt {
            1
        } else if shifted {
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
    min_sustain_ms: u32,
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
        Span::styled(" — Gerçek örneklenmiş piyano sesleri (TimGM6mb SoundFont)", Style::default().fg(Color::DarkGray)),
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

    // 4. Durum Paneli (3 Sütun: Notalar %50, Uzama %25, Pedal %25)
    let status_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(main_layout[3]);

    let status_text = if let Some((name, oct, midi)) = last_played_info {
        Line::from(vec![
            Span::styled(" ▶ Çalınan: ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{}{} ", name, oct), Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD)),
            Span::styled(format!("(#{})", midi), Style::default().fg(Color::DarkGray)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" Durum: ", Style::default().fg(Color::DarkGray)),
            Span::styled("Gerçek Akustik Piyano hazır. [←/→] ile uzama süresini ayarlayın.", Style::default().fg(Color::DarkGray)),
        ])
    };

    let status_box = Paragraph::new(status_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(if last_played_info.is_some() { Color::Green } else { Color::DarkGray })),
    );
    frame.render_widget(status_box, status_cols[0]);

    // Minimum Uzama Kutusu
    let time_box = Paragraph::new(Line::from(vec![
        Span::styled(" Uzama: ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{} ms ", min_sustain_ms), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
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
        Span::styled("SPACE (Basılı Tut)", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        Span::styled("  │  Uzama Ayarı: ", Style::default().fg(Color::DarkGray)),
        Span::styled("← / →", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
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
    // 1. SoundFont'u binary içine gömülü olarak yükle (program hangi klasörden çalışırsa çalışsın bulunur)
    static PIANO_SF2: &[u8] = include_bytes!("../assets/piano.sf2");
    let mut sf2_reader = std::io::Cursor::new(PIANO_SF2);
    let sound_font = Arc::new(SoundFont::new(&mut sf2_reader)?);

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

    // Kitty Keyboard Protocol:
    // - REPORT_EVENT_TYPES: donanımsal tuş bırakma (Release) olayları
    // - REPORT_ALL_KEYS_AS_ESCAPE_CODES: tuşlar klavye düzeninden bağımsız, fiziksel halleriyle gelir.
    //   Türkçe Q'da Shift+2 → "'" yerine "2 + SHIFT" olarak gelir; ölü tuş (^) sorunu da ortadan kalkar.
    let _ = stdout.execute(PushKeyboardEnhancementFlags(
        KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES,
    ));

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Aktif tuşlar ve nota takibi: (Slot bazında o an basılan MIDI notası)
    let mut active_midi_notes: [Option<i32>; 24] = [None; 24];
    let mut first_press_time: [Option<Instant>; 24] = Default::default(); // Tuşa ilk basılan an
    let mut last_repeat_time: [Option<Instant>; 24] = Default::default(); // Son tekrar anı (fallback timeout için)
    // Tuş bırakıldığında sesin hemen kesilmeyip uzamasını sağlayan zamanlayıcı:
    let mut pending_release_notes: Vec<(i32, Instant)> = Vec::new();
    let mut received_real_release = false;

    let pedal_atomic = Arc::new(AtomicBool::new(false));
    let mut last_space_time: Option<Instant> = None;
    let fallback_release_timeout = Duration::from_millis(750);

    // Minimum uzama süresi (varsayılan 550 ms)
    let mut min_sustain_ms: u32 = 550;

    let mut is_active_array = [false; 24];
    let mut last_played_info: Option<(String, i8, i32)> = None;

    loop {
        let pedal_held = pedal_atomic.load(Ordering::Relaxed);
        terminal.draw(|f| draw_ui(f, &is_active_array, &last_played_info, pedal_held, min_sustain_ms))?;

        let mut state_changed = false;

        if event::poll(Duration::from_millis(15))? {
            let ev = event::read()?;
            if let Event::Key(key_event) = ev {
                if key_event.code == KeyCode::Esc {
                    break;
                }

                // Sol ve Sağ Ok: Minimum Uzama Süresini Değiştir
                if key_event.kind == KeyEventKind::Press || key_event.kind == KeyEventKind::Repeat {
                    match key_event.code {
                        KeyCode::Left => {
                            if min_sustain_ms > 100 {
                                min_sustain_ms -= 50;
                                state_changed = true;
                            }
                        }
                        KeyCode::Right => {
                            if min_sustain_ms < 3000 {
                                min_sustain_ms += 50;
                                state_changed = true;
                            }
                        }
                        _ => {}
                    }
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
                        let now = Instant::now();
                        last_repeat_time[slot_idx] = Some(now);

                        if active_midi_notes[slot_idx].is_none() {
                            first_press_time[slot_idx] = Some(now);
                            let slot = SLOTS[slot_idx];
                            let midi_note = to_midi_note(slot.semitone, octave);

                            let mut s = synth.lock().unwrap();
                            // Aynı nota hâlâ uzama süresindeyse eski zamanlayıcıyı iptal et
                            pending_release_notes.retain(|(n, _)| *n != midi_note);
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
                            let start_time = first_press_time[slot_idx].unwrap_or_else(Instant::now);
                            let min_duration = Duration::from_millis(min_sustain_ms as u64);
                            let elapsed = Instant::now().duration_since(start_time);

                            // AKUSTİK DOĞRULUK:
                            // Eğer tuş zaten min_duration'dan daha uzun süredir basılı tutulduysa,
                            // parmak çekildiği an gerçek piyano gibi hemen susar!
                            // Eğer hızlıca dokunup çekildiyse (staccato), kalan süre kadar uzamaya devam eder.
                            if elapsed >= min_duration {
                                let mut s = synth.lock().unwrap();
                                s.note_off(0, midi_note);
                            } else {
                                let release_at = start_time + min_duration;
                                pending_release_notes.push((midi_note, release_at));
                            }

                            first_press_time[slot_idx] = None;
                            last_repeat_time[slot_idx] = None;
                            state_changed = true;
                        }
                    }
                }
            }
        }

        // Bekleyen uzama süreleri dolan notaların susturucusunu kapat
        let now = Instant::now();
        let mut i = 0;
        while i < pending_release_notes.len() {
            if now >= pending_release_notes[i].1 {
                let (note, _) = pending_release_notes.remove(i);
                let mut s = synth.lock().unwrap();
                s.note_off(0, note);
            } else {
                i += 1;
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
            for idx in 0..24 {
                if let Some(last_rep) = last_repeat_time[idx] {
                    if now.duration_since(last_rep) > fallback_release_timeout {
                        if let Some(midi_note) = active_midi_notes[idx].take() {
                            let start_time = first_press_time[idx].unwrap_or(last_rep);
                            let min_duration = Duration::from_millis(min_sustain_ms as u64);
                            let elapsed = now.duration_since(start_time);
                            if elapsed >= min_duration {
                                let mut s = synth.lock().unwrap();
                                s.note_off(0, midi_note);
                            } else {
                                pending_release_notes.push((midi_note, start_time + min_duration));
                            }
                        }
                        first_press_time[idx] = None;
                        last_repeat_time[idx] = None;
                        state_changed = true;
                    }
                }
            }
        }

        if state_changed {
            for idx in 0..24 {
                is_active_array[idx] = active_midi_notes[idx].is_some();
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
