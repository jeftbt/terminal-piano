use std::f32::consts::PI;
use std::io::stdout;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
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
        // Tuş bırakıldığında yumuşak sönümleme: 450ms
        let normal_release_samples = (sample_rate as f32 * 0.45) as usize;
        // Pedal bırakıldığında keçelerin tellere inip sönmesi: 700ms
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

        // Canlı ayarlanan minimum süre (örneğin 100ms - 1500ms)
        let min_ms = self.min_press_ms.load(Ordering::Relaxed);
        let min_press_samples = (self.sample_rate as f32 * (min_ms as f32 / 1000.0)) as usize;

        let min_duration_reached = self.sample_clock >= min_press_samples;

        // Release başladığı anda mevcut ses kazancını dondur (sıçramayı engelle)
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
            // Pedal basılıyken havada asılı kalma (4.5s)
            let sustain_samples = (self.sample_rate as f32 * 4.5) as usize;
            if self.sample_clock > sustain_samples {
                return None;
            }
            let natural_decay = (1.0 - (self.sample_clock as f32 / sustain_samples as f32)).powf(1.3);
            self.current_gain = natural_decay;
        } else {
            // Tuş basılı tutulurken Attack (ilk 12ms)
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

struct NoteDef {
    name: &'static str,
    key_label: &'static str,
    freq: f32,
}

const NOTES: [NoteDef; 7] = [
    NoteDef { name: "DO",  key_label: "N", freq: 261.63 },
    NoteDef { name: "RE",  key_label: "J", freq: 293.66 },
    NoteDef { name: "MI",  key_label: "I", freq: 329.63 },
    NoteDef { name: "FA",  key_label: "M", freq: 349.23 },
    NoteDef { name: "SOL", key_label: "K", freq: 392.00 },
    NoteDef { name: "LA",  key_label: "O", freq: 440.00 },
    NoteDef { name: "SI",  key_label: "P", freq: 493.88 },
];

fn map_key_to_index(code: KeyCode) -> Option<usize> {
    match code {
        KeyCode::Char('n') | KeyCode::Char('N') => Some(0),
        KeyCode::Char('j') | KeyCode::Char('J') => Some(1),
        KeyCode::Char('ı') | KeyCode::Char('I') | KeyCode::Char('i') | KeyCode::Char('İ') => Some(2),
        KeyCode::Char('m') | KeyCode::Char('M') => Some(3),
        KeyCode::Char('k') | KeyCode::Char('K') => Some(4),
        KeyCode::Char('o') | KeyCode::Char('O') => Some(5),
        KeyCode::Char('p') | KeyCode::Char('P') => Some(6),
        _ => None,
    }
}

fn draw_ui(
    frame: &mut ratatui::Frame,
    active_keys: &[bool; 7],
    pedal_pressed: bool,
    min_press_ms: u32,
) {
    let size = frame.area();

    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Başlık
            Constraint::Length(12), // Piyano Tuşları
            Constraint::Length(3),  // Durum Çubuğu + Pedal + Süre Ayarı
            Constraint::Length(2),  // Kısayol Bilgisi
        ])
        .margin(1)
        .split(size);

    // 1. Başlık Kutusu
    let header = Paragraph::new(Line::from(vec![
        Span::styled("🎹 ", Style::default()),
        Span::styled(
            "RUST TERMINAL PIANO",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" — Dinamik Uzama & Rezonans Kontrolü", Style::default().fg(Color::DarkGray)),
    ]))
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(header, main_layout[0]);

    // 2. Piyano Tuşları
    let piano_area = main_layout[1];
    let key_constraints = vec![Constraint::Ratio(1, 7); 7];
    let key_columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(key_constraints)
        .split(piano_area);

    for (i, note) in NOTES.iter().enumerate() {
        let is_active = active_keys[i];

        let (border_color, bg_color, fg_color) = if is_active {
            (Color::Green, Color::Green, Color::Black)
        } else {
            (Color::White, Color::Reset, Color::White)
        };

        let key_text = vec![
            Line::from(""),
            Line::from(Span::styled(
                note.name,
                Style::default()
                    .fg(fg_color)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!("[ {} ]", note.key_label),
                Style::default()
                    .fg(if is_active { Color::Black } else { Color::Yellow })
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("{:.0}Hz", note.freq),
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from(if is_active { "████" } else { "░░░░" }),
        ];

        let key_block = Paragraph::new(key_text)
            .alignment(Alignment::Center)
            .style(Style::default().bg(bg_color))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(border_color)),
            );

        frame.render_widget(key_block, key_columns[i]);
    }

    // 3. Durum Paneli (3 Sütun: Notalar %50, Süre Ayarı %25, Pedal %25)
    let status_area = main_layout[2];
    let status_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50), // Notalar
            Constraint::Percentage(25), // Min Uzama Süresi
            Constraint::Percentage(25), // Pedal Durumu
        ])
        .split(status_area);

    let active_names: Vec<String> = NOTES
        .iter()
        .enumerate()
        .filter(|(i, _)| active_keys[*i])
        .map(|(_, n)| format!("{} ({:.1}Hz)", n.name, n.freq))
        .collect();

    let status_line = if active_names.is_empty() {
        Line::from(vec![
            Span::styled(" Durum: ", Style::default().fg(Color::DarkGray)),
            Span::styled("Hazir. [← / →] ile uzama suresini degistirin.", Style::default().fg(Color::DarkGray)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" ▶ Caliniyor: ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(
                active_names.join(" + "),
                Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD),
            ),
        ])
    };

    let status_box = Paragraph::new(status_line).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(if active_names.is_empty() { Color::DarkGray } else { Color::Green })),
    );
    frame.render_widget(status_box, status_cols[0]);

    // Minimum Uzama Süresi Kutusu
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
            " PEDAL: AKTIF ",
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

    // 4. Alt Kısayol Bilgisi
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Notalar: ", Style::default().fg(Color::DarkGray)),
        Span::styled("N, J, I, M, K, O, P", Style::default().fg(Color::Yellow)),
        Span::styled("  │  Uzama Ayari: ", Style::default().fg(Color::DarkGray)),
        Span::styled("Sol/Sag Ok (← / →)", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled("  │  Pedal: ", Style::default().fg(Color::DarkGray)),
        Span::styled("SPACE", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        Span::styled("  │  Cikis: ", Style::default().fg(Color::DarkGray)),
        Span::styled("Q / ESC", Style::default().fg(Color::Red)),
    ]))
    .alignment(Alignment::Center);
    frame.render_widget(footer, main_layout[3]);
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

    let mut active_flags: [Option<Arc<AtomicBool>>; 7] = Default::default();
    let mut last_press_time: [Option<Instant>; 7] = Default::default();
    let mut received_real_release = false;

    // Sustain Pedal Durumu
    let pedal_atomic = Arc::new(AtomicBool::new(false));
    let mut last_space_time: Option<Instant> = None;

    // Dinamik Minimum Uzama Süresi (Varsayılan 500 ms)
    let min_press_atomic = Arc::new(AtomicU32::new(500));

    let fallback_release_timeout = Duration::from_millis(750);
    let mut is_active_array = [false; 7];

    loop {
        let pedal_held = pedal_atomic.load(Ordering::Relaxed);
        let current_min_ms = min_press_atomic.load(Ordering::Relaxed);
        terminal.draw(|f| draw_ui(f, &is_active_array, pedal_held, current_min_ms))?;

        let mut state_changed = false;

        if event::poll(Duration::from_millis(15))? {
            let ev = event::read()?;
            if let Event::Key(key_event) = ev {
                // Çıkış kontrolleri
                if key_event.code == KeyCode::Char('q')
                    || key_event.code == KeyCode::Char('Q')
                    || key_event.code == KeyCode::Esc
                {
                    break;
                }

                // Sol ve Sağ Ok Tuşları: Minimum Uzama Süresini Ayarla (50ms adımlarla)
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

                // Notalar
                if let Some(idx) = map_key_to_index(key_event.code) {
                    if key_event.kind == KeyEventKind::Press || key_event.kind == KeyEventKind::Repeat {
                        last_press_time[idx] = Some(Instant::now());

                        if active_flags[idx].is_none() {
                            let flag = Arc::new(AtomicBool::new(true));
                            let source = ContinuousTone::new(
                                NOTES[idx].freq,
                                44100,
                                Arc::clone(&flag),
                                Arc::clone(&pedal_atomic),
                                Arc::clone(&min_press_atomic),
                            );
                            if let Ok(sink) = Sink::try_new(&stream_handle) {
                                sink.append(source);
                                sink.detach();
                                active_flags[idx] = Some(flag);
                                state_changed = true;
                            }
                        }
                    } else if key_event.kind == KeyEventKind::Release {
                        received_real_release = true;
                        if let Some(flag) = active_flags[idx].take() {
                            flag.store(false, Ordering::Relaxed);
                            last_press_time[idx] = None;
                            state_changed = true;
                        }
                    }
                }
            }
        }

        // Space (Pedal) Fallback zaman aşımı kontrolü
        if !received_real_release && pedal_atomic.load(Ordering::Relaxed) {
            if let Some(space_time) = last_space_time {
                if Instant::now().duration_since(space_time) > fallback_release_timeout {
                    pedal_atomic.store(false, Ordering::Relaxed);
                    last_space_time = None;
                    state_changed = true;
                }
            }
        }

        // Nota tuşları Fallback zaman aşımı kontrolü
        if !received_real_release {
            let now = Instant::now();
            for i in 0..7 {
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
            for i in 0..7 {
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
