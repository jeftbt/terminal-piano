use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Stroke, Vec2};
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
            buffer_pos: 256,
        }
    }
}

impl Iterator for SoundFontSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
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
        2
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[derive(Clone, Copy, Debug)]
struct KeySlot {
    semitone: i8, // 0..11 (C, C#, D, D#, E, F, F#, G, G#, A, A#, B)
    key_label: &'static str,
    key_code: egui::Key,
}

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

// 24 Tuşluk Slot Listesi
const SLOTS: [KeySlot; 24] = [
    // --- ALT SIRA (Oktav 3 Bazlı) ---
    KeySlot { semitone: 0,  key_label: "Z", key_code: egui::Key::Z }, // C3
    KeySlot { semitone: 1,  key_label: "S", key_code: egui::Key::S }, // C#3
    KeySlot { semitone: 2,  key_label: "X", key_code: egui::Key::X }, // D3
    KeySlot { semitone: 3,  key_label: "D", key_code: egui::Key::D }, // D#3
    KeySlot { semitone: 4,  key_label: "C", key_code: egui::Key::C }, // E3
    KeySlot { semitone: 5,  key_label: "V", key_code: egui::Key::V }, // F3
    KeySlot { semitone: 6,  key_label: "G", key_code: egui::Key::G }, // F#3
    KeySlot { semitone: 7,  key_label: "B", key_code: egui::Key::B }, // G3
    KeySlot { semitone: 8,  key_label: "H", key_code: egui::Key::H }, // G#3
    KeySlot { semitone: 9,  key_label: "N", key_code: egui::Key::N }, // A3
    KeySlot { semitone: 10, key_label: "J", key_code: egui::Key::J }, // A#3
    KeySlot { semitone: 11, key_label: "M", key_code: egui::Key::M }, // B3

    // --- ÜST SIRA (Oktav 4 Bazlı) ---
    KeySlot { semitone: 0,  key_label: "Q", key_code: egui::Key::Q }, // C4
    KeySlot { semitone: 1,  key_label: "2", key_code: egui::Key::Num2 }, // C#4
    KeySlot { semitone: 2,  key_label: "W", key_code: egui::Key::W }, // D4
    KeySlot { semitone: 3,  key_label: "3", key_code: egui::Key::Num3 }, // D#4
    KeySlot { semitone: 4,  key_label: "E", key_code: egui::Key::E }, // E4
    KeySlot { semitone: 5,  key_label: "R", key_code: egui::Key::R }, // F4
    KeySlot { semitone: 6,  key_label: "5", key_code: egui::Key::Num5 }, // F#4
    KeySlot { semitone: 7,  key_label: "T", key_code: egui::Key::T }, // G4
    KeySlot { semitone: 8,  key_label: "6", key_code: egui::Key::Num6 }, // G#4
    KeySlot { semitone: 9,  key_label: "Y", key_code: egui::Key::Y }, // A4
    KeySlot { semitone: 10, key_label: "7", key_code: egui::Key::Num7 }, // A#4
    KeySlot { semitone: 11, key_label: "U", key_code: egui::Key::U }, // B4
];

fn to_midi_note(semitone: i8, octave: i8) -> i32 {
    ((octave + 1) * 12 + semitone) as i32
}

fn velocity_label(vel: i32) -> &'static str {
    match vel {
        0..=35 => "pp (Pianissimo)",
        36..=60 => "p (Piano)",
        61..=85 => "mp (Mezzo-Piano)",
        86..=105 => "mf (Mezzo-Forte)",
        106..=120 => "f (Forte)",
        _ => "ff (Fortissimo)",
    }
}

struct PianoApp {
    synth: Arc<Mutex<Synthesizer>>,
    _stream: OutputStream,
    _sink: Sink,

    active_midi_notes: [Option<i32>; 24],
    first_press_time: [Option<Instant>; 24],
    pending_release_notes: Vec<(i32, Instant)>,

    pedal_pressed: bool,
    min_sustain_ms: u32,
    velocity: i32,

    last_played_info: Option<(String, i8, i32)>,
    mouse_active_slot: Option<usize>,
}

impl PianoApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        static PIANO_SF2: &[u8] = include_bytes!("../assets/piano.sf2");
        let mut sf2_reader = std::io::Cursor::new(PIANO_SF2);
        let sound_font = Arc::new(SoundFont::new(&mut sf2_reader).expect("SoundFont yüklenemedi"));

        let sample_rate = 44100;
        let settings = SynthesizerSettings::new(sample_rate as i32);
        let synth = Arc::new(Mutex::new(Synthesizer::new(&sound_font, &settings).expect("Synth başlatılamadı")));

        let (stream, stream_handle) = OutputStream::try_default().expect("Ses çıkış aygıtı bulunamadı");
        let sink = Sink::try_new(&stream_handle).expect("Sink oluşturulamadı");
        let source = SoundFontSource::new(Arc::clone(&synth), sample_rate);
        sink.append(source);
        sink.play();

        Self {
            synth,
            _stream: stream,
            _sink: sink,
            active_midi_notes: [None; 24],
            first_press_time: Default::default(),
            pending_release_notes: Vec::new(),
            pedal_pressed: false,
            min_sustain_ms: 550,
            velocity: 95,
            last_played_info: None,
            mouse_active_slot: None,
        }
    }

    fn play_slot(&mut self, slot_idx: usize, octave: i8) {
        if self.active_midi_notes[slot_idx].is_none() {
            let now = Instant::now();
            self.first_press_time[slot_idx] = Some(now);

            let slot = SLOTS[slot_idx];
            let midi_note = to_midi_note(slot.semitone, octave);

            let mut s = self.synth.lock().unwrap();
            self.pending_release_notes.retain(|(n, _)| *n != midi_note);
            s.note_on(0, midi_note, self.velocity);

            self.active_midi_notes[slot_idx] = Some(midi_note);
            self.last_played_info = Some((
                NOTE_NAMES[slot.semitone as usize].to_string(),
                octave,
                midi_note,
            ));
        }
    }

    fn release_slot(&mut self, slot_idx: usize) {
        if let Some(midi_note) = self.active_midi_notes[slot_idx].take() {
            let start_time = self.first_press_time[slot_idx].unwrap_or_else(Instant::now);
            let min_duration = Duration::from_millis(self.min_sustain_ms as u64);
            let elapsed = Instant::now().duration_since(start_time);

            // Tiz Dampersiz Bölge (MIDI >= 90 / F#6+)
            let is_damperless_treble = midi_note >= 90;

            if !is_damperless_treble {
                if elapsed < min_duration {
                    let release_at = start_time + min_duration;
                    self.pending_release_notes.push((midi_note, release_at));
                } else {
                    let mut s = self.synth.lock().unwrap();
                    s.note_off(0, midi_note);
                }
            }
        }
    }

    fn draw_keyboard(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        slot_offset: usize,
        base_octave: i8,
        active_shift: bool,
        active_alt: bool,
    ) {
        let octave = if active_alt {
            if slot_offset >= 12 { 6 } else { 1 }
        } else if active_shift {
            if slot_offset >= 12 { 5 } else { 2 }
        } else {
            base_octave
        };

        ui.group(|ui| {
            ui.heading(title);
            ui.add_space(4.0);

            let available_width = ui.available_width();
            let key_width = (available_width / 7.0).min(110.0);
            let white_height = 200.0;
            let black_width = key_width * 0.58;
            let black_height = white_height * 0.60;

            let (response, painter) = ui.allocate_painter(
                Vec2::new(key_width * 7.0, white_height + 4.0),
                egui::Sense::click_and_drag(),
            );

            let origin = response.rect.min;

            // 1. Beyaz Tuşlar Çizimi
            let white_slots = [0, 2, 4, 5, 7, 9, 11];
            for (i, &semi_idx) in white_slots.iter().enumerate() {
                let slot_idx = slot_offset + semi_idx;
                let is_active = self.active_midi_notes[slot_idx].is_some();
                let key_rect = Rect::from_min_size(
                    Pos2::new(origin.x + (i as f32) * key_width, origin.y),
                    Vec2::new(key_width, white_height),
                );

                let bg_color = if is_active {
                    Color32::from_rgb(46, 204, 113) // Aktif Yeşil
                } else {
                    Color32::from_rgb(248, 248, 246) // Fildişi Beyazı
                };

                // Tuş gövdesi
                painter.rect_filled(
                    key_rect,
                    CornerRadius { nw: 0, ne: 0, sw: 5, se: 5 },
                    bg_color,
                );
                // Tuş kenarlığı
                painter.rect_stroke(
                    key_rect,
                    CornerRadius { nw: 0, ne: 0, sw: 5, se: 5 },
                    Stroke::new(1.0, Color32::from_rgb(180, 180, 180)),
                    egui::StrokeKind::Inside,
                );

                // Tuş Notası & Kısayol Etiketi
                let slot = SLOTS[slot_idx];
                let text_color = if is_active { Color32::BLACK } else { Color32::from_rgb(30, 30, 30) };
                
                painter.text(
                    Pos2::new(key_rect.center().x, key_rect.bottom() - 38.0),
                    egui::Align2::CENTER_CENTER,
                    format!("{}{}", NOTE_NAMES[slot.semitone as usize], octave),
                    egui::FontId::proportional(16.0),
                    text_color,
                );
                painter.text(
                    Pos2::new(key_rect.center().x, key_rect.bottom() - 16.0),
                    egui::Align2::CENTER_CENTER,
                    format!("[{}]", slot.key_label),
                    egui::FontId::monospace(14.0),
                    if is_active { Color32::from_rgb(10, 50, 20) } else { Color32::from_rgb(120, 120, 120) },
                );
            }

            // 2. Siyah Tuşlar Çizimi
            let black_slots = [
                (1, 0), // C#
                (3, 1), // D#
                (6, 3), // F#
                (8, 4), // G#
                (10, 5),// A#
            ];

            for &(semi_idx, col_idx) in &black_slots {
                let slot_idx = slot_offset + semi_idx;
                let is_active = self.active_midi_notes[slot_idx].is_some();
                let center_x = origin.x + ((col_idx + 1) as f32) * key_width;
                let key_rect = Rect::from_min_size(
                    Pos2::new(center_x - black_width / 2.0, origin.y),
                    Vec2::new(black_width, black_height),
                );

                let bg_color = if is_active {
                    Color32::from_rgb(46, 204, 113)
                } else {
                    Color32::from_rgb(26, 26, 30) // Abanoz Siyahı
                };

                // Siyah tuş gölgesi
                let shadow_rect = Rect::from_min_size(
                    Pos2::new(key_rect.left() + 2.0, key_rect.top() + 2.0),
                    Vec2::new(black_width, black_height + 3.0),
                );
                painter.rect_filled(shadow_rect, CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 }, Color32::from_black_alpha(80));

                painter.rect_filled(
                    key_rect,
                    CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 },
                    bg_color,
                );
                painter.rect_stroke(
                    key_rect,
                    CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 },
                    Stroke::new(1.0, Color32::from_rgb(60, 60, 65)),
                    egui::StrokeKind::Inside,
                );

                let slot = SLOTS[slot_idx];
                let text_color = if is_active { Color32::BLACK } else { Color32::WHITE };

                painter.text(
                    Pos2::new(key_rect.center().x, key_rect.bottom() - 30.0),
                    egui::Align2::CENTER_CENTER,
                    NOTE_NAMES[slot.semitone as usize],
                    egui::FontId::proportional(13.0),
                    text_color,
                );
                painter.text(
                    Pos2::new(key_rect.center().x, key_rect.bottom() - 12.0),
                    egui::Align2::CENTER_CENTER,
                    format!("[{}]", slot.key_label),
                    egui::FontId::monospace(12.0),
                    if is_active { Color32::BLACK } else { Color32::from_rgb(200, 200, 200) },
                );
            }

            // Fare Tıklama / Sürükleme Desteği
            if response.hovered() && ui.input(|i| i.pointer.primary_down()) {
                if let Some(pos) = response.hover_pos() {
                    let mut hit_slot: Option<usize> = None;

                    // Önce siyah tuşlar kontrol edilir (üstte oldukları için)
                    for &(semi_idx, col_idx) in &black_slots {
                        let center_x = origin.x + ((col_idx + 1) as f32) * key_width;
                        let rect = Rect::from_min_size(
                            Pos2::new(center_x - black_width / 2.0, origin.y),
                            Vec2::new(black_width, black_height),
                        );
                        if rect.contains(pos) {
                            hit_slot = Some(slot_offset + semi_idx);
                            break;
                        }
                    }

                    // Siyah tuşa basılmadıysa beyaz tuşlar
                    if hit_slot.is_none() {
                        let col = ((pos.x - origin.x) / key_width).floor() as usize;
                        if col < 7 {
                            hit_slot = Some(slot_offset + white_slots[col]);
                        }
                    }

                    if let Some(slot) = hit_slot {
                        if self.mouse_active_slot != Some(slot) {
                            if let Some(prev) = self.mouse_active_slot {
                                self.release_slot(prev);
                            }
                            self.play_slot(slot, octave);
                            self.mouse_active_slot = Some(slot);
                        }
                    }
                }
            } else if let Some(prev) = self.mouse_active_slot {
                self.release_slot(prev);
                self.mouse_active_slot = None;
            }
        });
    }
}

impl eframe::App for PianoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();

        // 1. Süresi dolan uzama (sustain) notalarını durdur
        let mut expired = Vec::new();
        self.pending_release_notes.retain(|(midi, release_time)| {
            if now >= *release_time {
                expired.push(*midi);
                false
            } else {
                true
            }
        });

        if !expired.is_empty() {
            let mut s = self.synth.lock().unwrap();
            for note in expired {
                s.note_off(0, note);
            }
        }

        // 2. Klavye Girdileri
        let (shift_pressed, alt_pressed) = ctx.input(|i| {
            (i.modifiers.shift, i.modifiers.alt)
        });

        // Tuşe (Velocity) Kontrolü (Yukarı / Aşağı Ok)
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            if self.velocity <= 122 {
                self.velocity += 5;
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            if self.velocity >= 25 {
                self.velocity -= 5;
            }
        }

        // Uzama Süresi (Sol / Sağ Ok)
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            if self.min_sustain_ms > 100 {
                self.min_sustain_ms -= 50;
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            if self.min_sustain_ms < 3000 {
                self.min_sustain_ms += 50;
            }
        }

        // Sustain Pedalı (Space)
        let space_down = ctx.input(|i| i.key_down(egui::Key::Space));
        if space_down != self.pedal_pressed {
            self.pedal_pressed = space_down;
            let mut s = self.synth.lock().unwrap();
            s.process_midi_message(0, 0xB0, 64, if space_down { 127 } else { 0 });
        }

        // Kromatik 24 Tuş Kontrolü
        for (slot_idx, slot) in SLOTS.iter().enumerate() {
            let is_down = ctx.input(|i| i.key_down(slot.key_code));
            let was_down = self.active_midi_notes[slot_idx].is_some();

            let octave = if slot_idx >= 12 {
                if alt_pressed { 6 } else if shift_pressed { 5 } else { 4 }
            } else {
                if alt_pressed { 1 } else if shift_pressed { 2 } else { 3 }
            };

            if is_down && !was_down {
                self.play_slot(slot_idx, octave);
            } else if !is_down && was_down {
                self.release_slot(slot_idx);
            }
        }

        // 3. UI Çizimi
        ui.horizontal(|ui| {
            ui.heading("🎹 ACOUSTIC GRAND PIANO (SoundFont Engine)");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.pedal_pressed {
                    ui.colored_label(Color32::from_rgb(255, 105, 180), "PEDAL: AKTİF [SPACE]");
                } else {
                    ui.colored_label(Color32::GRAY, "PEDAL: KAPALI [SPACE]");
                }
            });
        });

        ui.separator();

        // Kontrol Paneli (Tuşe, Uzama, Çalınan Nota)
        ui.horizontal(|ui| {
            ui.label(format!("Tuşe (Velocity): {} [{}]", self.velocity, velocity_label(self.velocity)));
            ui.add(egui::Slider::new(&mut self.velocity, 20..=127).text("↑/↓"));

            ui.separator();

            ui.label(format!("Min Uzama: {} ms", self.min_sustain_ms));
            ui.add(egui::Slider::new(&mut self.min_sustain_ms, 100..=3000).text("←/→"));

            ui.separator();

            if let Some((name, oct, midi)) = &self.last_played_info {
                ui.colored_label(
                    Color32::from_rgb(46, 204, 113),
                    format!("▶ Çalınan: {}{} (#{})", name, oct, midi),
                );
            } else {
                ui.colored_label(Color32::GRAY, "Hazır — Tuşlara basın veya fareyle tıklayın");
            }
        });

        ui.add_space(8.0);

        // Üst Sıra (Tiz Grup)
        self.draw_keyboard(
            ui,
            "🎼 Tiz Grup [Q..U] (Varsayılan: Oktav 4 │ Shift: Oktav 5 │ Alt: Oktav 6 Dampersiz)",
            12,
            4,
            shift_pressed,
            alt_pressed,
        );

        ui.add_space(8.0);

        // Alt Sıra (Bas Grup)
        self.draw_keyboard(
            ui,
            "🎵 Bas Grup [Z..M] (Varsayılan: Oktav 3 │ Shift: Oktav 2 │ Alt: Oktav 1 Derin Bas)",
            0,
            3,
            shift_pressed,
            alt_pressed,
        );

        ui.add_space(6.0);
        ui.separator();

        // Alt Bilgi / Kısayollar
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Kısayollar:").strong());
            ui.label("Piyano Tuşları: [Z..M] & [Q..U]  │  Siyah Tuşlar: [S, D, G, H, J] & [2, 3, 5, 6, 7]");
            ui.label("│  Pedal: [Space]  │  Oktavlar: [Shift / Alt]  │  Tuşe: [↑ / ↓]  │  Uzama: [← / →]");
        });

        // 60 FPS pürüzsüz animasyon / tuş tepkisi
        ctx.request_repaint();
    }
}

fn main() -> Result<(), eframe::Error> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Acoustic Grand Piano (egui GUI)")
            .with_inner_size([880.0, 680.0])
            .with_min_inner_size([700.0, 550.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Acoustic Grand Piano",
        native_options,
        Box::new(|cc| Ok(Box::new(PianoApp::new(cc)))),
    )
}
