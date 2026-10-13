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
    semitone: i8, // 0..11
    key_label: &'static str,
    key_code: egui::Key,
}

const NOTE_NAMES_LETTERS: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

const NOTE_NAMES_SOLFEGE: [&str; 12] = [
    "Do", "Do#", "Re", "Re#", "Mi", "Fa", "Fa#", "Sol", "Sol#", "La", "La#", "Si",
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
    base_octave: i8, // Varsayılan 3 (yani 3 ve 4). Tab ile 1, 2, 3, 4, 5 arasında döner.
    use_letter_notation: bool, // true: A-B-C-D-E-F-G (Harf) modu, false: Do-Re-Mi modu

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
            base_octave: 3,
            use_letter_notation: true, // Varsayılan A B C D E F G
            last_played_info: None,
            mouse_active_slot: None,
        }
    }

    fn note_name(&self, semitone: i8) -> &'static str {
        let idx = (semitone as usize) % 12;
        if self.use_letter_notation {
            NOTE_NAMES_LETTERS[idx]
        } else {
            NOTE_NAMES_SOLFEGE[idx]
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
                self.note_name(slot.semitone).to_string(),
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

    fn draw_continuous_piano(&mut self, ui: &mut egui::Ui, shift_pressed: bool, alt_pressed: bool) {
        let bass_octave = (self.base_octave + if alt_pressed { -2 } else if shift_pressed { -1 } else { 0 }).clamp(0, 8);
        let treble_octave = (self.base_octave + 1 + if alt_pressed { 2 } else if shift_pressed { 1 } else { 0 }).clamp(0, 8);

        let available_width = ui.available_width().max(800.0);
        let total_white_keys = 14.0;
        let white_width = (available_width / total_white_keys).clamp(52.0, 72.0);
        let white_height = 240.0;
        let black_width = white_width * 0.58;
        let black_height = white_height * 0.62;

        let total_piano_width = white_width * total_white_keys;
        let left_padding = ((ui.available_width() - total_piano_width) / 2.0).max(0.0);

        // Piyano Çerçevesi (Grand Piano Wooden Bed & Red Felt)
        let frame_height = white_height + 40.0;
        let (outer_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), frame_height), egui::Sense::hover());
        let painter = ui.painter_at(outer_rect);

        let bed_rect = Rect::from_min_size(
            Pos2::new(outer_rect.min.x + left_padding - 14.0, outer_rect.min.y + 4.0),
            Vec2::new(total_piano_width + 28.0, frame_height - 8.0),
        );

        // Abanoz Lake Kasa (Piano Finish)
        painter.rect_filled(bed_rect, CornerRadius::same(12), Color32::from_rgb(12, 13, 16));
        painter.rect_stroke(bed_rect, CornerRadius::same(12), Stroke::new(1.5, Color32::from_rgb(38, 40, 48)), egui::StrokeKind::Inside);

        // Altın İnce Hat (Brass Inlay)
        painter.line_segment(
            [
                Pos2::new(bed_rect.left() + 18.0, bed_rect.top() + 7.0),
                Pos2::new(bed_rect.right() - 18.0, bed_rect.top() + 7.0),
            ],
            Stroke::new(1.0, Color32::from_rgb(180, 140, 60)),
        );

        // Akustik Kırmızı Keçe (Red Damper Felt Strip)
        let felt_rect = Rect::from_min_size(
            Pos2::new(outer_rect.min.x + left_padding, bed_rect.top() + 14.0),
            Vec2::new(total_piano_width, 10.0),
        );
        painter.rect_filled(felt_rect, CornerRadius::same(2), Color32::from_rgb(180, 24, 38));

        let origin = Pos2::new(outer_rect.min.x + left_padding, felt_rect.bottom());

        // Etkileşim Alanı (Response)
        let keys_rect = Rect::from_min_size(origin, Vec2::new(total_piano_width, white_height));
        let keyboard_response = ui.interact(keys_rect, ui.id().with("piano_keyboard"), egui::Sense::click_and_drag());

        // 14 Beyaz Tuş Tanımı: (slot_idx, semitone, octave)
        let white_keys_data: [(usize, i8, i8); 14] = [
            // Bas Oktav (3)
            (0, 0, bass_octave),   // C
            (2, 2, bass_octave),   // D
            (4, 4, bass_octave),   // E
            (5, 5, bass_octave),   // F
            (7, 7, bass_octave),   // G
            (9, 9, bass_octave),   // A
            (11, 11, bass_octave), // B
            // Tiz Oktav (4)
            (12, 0, treble_octave),   // C
            (14, 2, treble_octave),   // D
            (16, 4, treble_octave),   // E
            (17, 5, treble_octave),   // F
            (19, 7, treble_octave),   // G
            (21, 9, treble_octave),   // A
            (23, 11, treble_octave),  // B
        ];

        // 10 Siyah Tuş Tanımı: (slot_idx, boundary_col, semitone, octave)
        let black_keys_data: [(usize, usize, i8, i8); 10] = [
            // Bas Oktav (3)
            (1, 1, 1, bass_octave),   // C#
            (3, 2, 3, bass_octave),   // D#
            (6, 4, 6, bass_octave),   // F#
            (8, 5, 8, bass_octave),   // G#
            (10, 6, 10, bass_octave), // A#
            // Tiz Oktav (4)
            (13, 8, 1, treble_octave),   // C#
            (15, 9, 3, treble_octave),   // D#
            (18, 11, 6, treble_octave),  // F#
            (20, 12, 8, treble_octave),  // G#
            (22, 13, 10, treble_octave), // A#
        ];

        // --- 1. BEYAZ TUŞLARI ÇİZ ---
        for (i, &(slot_idx, semi, oct)) in white_keys_data.iter().enumerate() {
            let is_active = self.active_midi_notes[slot_idx].is_some();
            let key_x = origin.x + (i as f32) * white_width;
            let key_y = if is_active { origin.y + 3.0 } else { origin.y }; // Basılınca hafif çökme
            let current_height = if is_active { white_height - 3.0 } else { white_height };

            let key_rect = Rect::from_min_size(
                Pos2::new(key_x, key_y),
                Vec2::new(white_width - 1.0, current_height),
            );

            // Fildişi Gradyan Hissi
            let key_fill = if is_active {
                Color32::from_rgb(113, 112, 255) // Linear Accent Violet Glow
            } else {
                Color32::from_rgb(250, 250, 248) // Saf Fildişi
            };

            painter.rect_filled(
                key_rect,
                CornerRadius { nw: 0, ne: 0, sw: 5, se: 5 },
                key_fill,
            );

            // Tuş Ön Kenar Bevel'i (3D ivory lip)
            let lip_rect = Rect::from_min_size(
                Pos2::new(key_rect.left(), key_rect.bottom() - 10.0),
                Vec2::new(key_rect.width(), 10.0),
            );
            painter.rect_filled(
                lip_rect,
                CornerRadius { nw: 0, ne: 0, sw: 5, se: 5 },
                if is_active { Color32::from_rgb(94, 106, 210) } else { Color32::from_rgb(230, 228, 222) },
            );

            // İnce Key Separation Çizgisi
            painter.rect_stroke(
                key_rect,
                CornerRadius { nw: 0, ne: 0, sw: 5, se: 5 },
                Stroke::new(1.0, Color32::from_rgb(190, 188, 180)),
                egui::StrokeKind::Inside,
            );

            // Tuş Etiketleri (Modern Minimal Tipografi)
            let slot = SLOTS[slot_idx];
            let note_color = if is_active { Color32::WHITE } else { Color32::from_rgb(40, 42, 48) };
            let key_bind_color = if is_active { Color32::from_rgb(215, 218, 255) } else { Color32::from_rgb(150, 154, 165) };

            painter.text(
                Pos2::new(key_rect.center().x, key_rect.bottom() - 44.0),
                egui::Align2::CENTER_CENTER,
                format!("{}{}", self.note_name(semi), oct),
                egui::FontId::proportional(15.0),
                note_color,
            );

            // Klavye Kısayol Tuşu Rozeti (Keyboard Key Pill)
            let pill_rect = Rect::from_center_size(
                Pos2::new(key_rect.center().x, key_rect.bottom() - 20.0),
                Vec2::new(26.0, 18.0),
            );
            painter.rect_filled(
                pill_rect,
                CornerRadius::same(4),
                if is_active { Color32::from_rgb(80, 90, 190) } else { Color32::from_rgb(238, 238, 235) },
            );
            painter.text(
                pill_rect.center(),
                egui::Align2::CENTER_CENTER,
                slot.key_label,
                egui::FontId::monospace(12.0),
                key_bind_color,
            );
        }

        // --- 2. SİYAH TUŞLARI ÇİZ (3D Ebony Keys) ---
        for &(slot_idx, col_idx, semi, _) in &black_keys_data {
            let is_active = self.active_midi_notes[slot_idx].is_some();
            let center_x = origin.x + (col_idx as f32) * white_width;
            let key_x = center_x - (black_width / 2.0);
            let key_y = if is_active { origin.y + 2.0 } else { origin.y };
            let current_height = if is_active { black_height - 2.0 } else { black_height };

            let key_rect = Rect::from_min_size(
                Pos2::new(key_x, key_y),
                Vec2::new(black_width, current_height),
            );

            // 3D Düşen Gölge (Drop Shadow onto white keys)
            let shadow_rect = Rect::from_min_size(
                Pos2::new(key_rect.left() - 2.0, key_rect.bottom()),
                Vec2::new(key_rect.width() + 4.0, 7.0),
            );
            painter.rect_filled(shadow_rect, CornerRadius::same(3), Color32::from_black_alpha(85));

            // Siyah Tuş Gövdesi
            let fill_color = if is_active {
                Color32::from_rgb(130, 143, 255) // Active Accent Violet
            } else {
                Color32::from_rgb(22, 23, 27) // Mat Abanoz
            };

            painter.rect_filled(
                key_rect,
                CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 },
                fill_color,
            );

            // Siyah Tuş Ön Eğim Yüzeyi (Top Highlight Bevel)
            let top_highlight = Rect::from_min_size(
                Pos2::new(key_rect.left() + 2.0, key_rect.top() + 1.0),
                Vec2::new(key_rect.width() - 4.0, key_rect.height() - 14.0),
            );
            painter.rect_filled(
                top_highlight,
                CornerRadius::same(2),
                if is_active { Color32::from_rgb(113, 112, 255) } else { Color32::from_rgb(34, 35, 42) },
            );

            painter.rect_stroke(
                key_rect,
                CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 },
                Stroke::new(1.0, Color32::from_rgb(50, 52, 60)),
                egui::StrokeKind::Inside,
            );

            let slot = SLOTS[slot_idx];
            painter.text(
                Pos2::new(key_rect.center().x, key_rect.bottom() - 32.0),
                egui::Align2::CENTER_CENTER,
                self.note_name(semi),
                egui::FontId::proportional(12.0),
                if is_active { Color32::WHITE } else { Color32::from_rgb(230, 230, 235) },
            );

            // Kısayol Rozeti
            let pill_rect = Rect::from_center_size(
                Pos2::new(key_rect.center().x, key_rect.bottom() - 14.0),
                Vec2::new(20.0, 16.0),
            );
            painter.rect_filled(
                pill_rect,
                CornerRadius::same(3),
                if is_active { Color32::from_rgb(90, 100, 200) } else { Color32::from_rgb(14, 15, 18) },
            );
            painter.text(
                pill_rect.center(),
                egui::Align2::CENTER_CENTER,
                slot.key_label,
                egui::FontId::monospace(11.0),
                if is_active { Color32::WHITE } else { Color32::from_rgb(170, 172, 180) },
            );
        }

        // --- FARE VE SÜRÜKLEME (MOUSE GLISSANDO) ETKİLEŞİMİ ---
        if keyboard_response.hovered() && ui.input(|i| i.pointer.primary_down()) {
            if let Some(pos) = keyboard_response.hover_pos() {
                let mut hit_slot: Option<(usize, i8)> = None;

                // Önce Siyah Tuşları Tara
                for &(slot_idx, col_idx, _, oct) in &black_keys_data {
                    let center_x = origin.x + (col_idx as f32) * white_width;
                    let rect = Rect::from_min_size(
                        Pos2::new(center_x - black_width / 2.0, origin.y),
                        Vec2::new(black_width, black_height),
                    );
                    if rect.contains(pos) {
                        hit_slot = Some((slot_idx, oct));
                        break;
                    }
                }

                // Siyah tuşa basılmadıysa Beyaz Tuşları Tara
                if hit_slot.is_none() {
                    let col = ((pos.x - origin.x) / white_width).floor() as usize;
                    if col < 14 {
                        let (slot_idx, _, oct) = white_keys_data[col];
                        hit_slot = Some((slot_idx, oct));
                    }
                }

                if let Some((slot, oct)) = hit_slot {
                    if self.mouse_active_slot != Some(slot) {
                        if let Some(prev) = self.mouse_active_slot {
                            self.release_slot(prev);
                        }
                        self.play_slot(slot, oct);
                        self.mouse_active_slot = Some(slot);
                    }
                }
            }
        } else if let Some(prev) = self.mouse_active_slot {
            self.release_slot(prev);
            self.mouse_active_slot = None;
        }
    }
}

impl eframe::App for PianoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();

        // 1. Süresi dolan uzama (sustain) notalarını kapat
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
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) && self.velocity <= 122 {
            self.velocity += 5;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) && self.velocity >= 25 {
            self.velocity -= 5;
        }

        // Uzama Süresi (Sol / Sağ Ok)
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) && self.min_sustain_ms > 100 {
            self.min_sustain_ms -= 50;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) && self.min_sustain_ms < 3000 {
            self.min_sustain_ms += 50;
        }

        // Sustain Pedalı (Space)
        let space_down = ctx.input(|i| i.key_down(egui::Key::Space));
        if space_down != self.pedal_pressed {
            self.pedal_pressed = space_down;
            let mut s = self.synth.lock().unwrap();
            s.process_midi_message(0, 0xB0, 64, if space_down { 127 } else { 0 });
        }

        // TAB: Piyano Temel Oktavını Değiştir (Döngüsel: 1..=5)
        if ctx.input(|i| i.key_pressed(egui::Key::Tab)) {
            // 1, 2, 3, 4, 5 arasında döngü
            self.base_octave = if self.base_octave >= 5 {
                1
            } else {
                self.base_octave + 1
            };
        }

        // Kromatik 24 Tuş Kontrolü
        for (slot_idx, slot) in SLOTS.iter().enumerate() {
            let is_down = ctx.input(|i| i.key_down(slot.key_code));
            let was_down = self.active_midi_notes[slot_idx].is_some();

            let octave = if slot_idx >= 12 {
                (self.base_octave + 1 + if alt_pressed { 2 } else if shift_pressed { 1 } else { 0 }).clamp(0, 8)
            } else {
                (self.base_octave + if alt_pressed { -2 } else if shift_pressed { -1 } else { 0 }).clamp(0, 8)
            };

            if is_down && !was_down {
                self.play_slot(slot_idx, octave);
            } else if !is_down && was_down {
                self.release_slot(slot_idx);
            }
        }

        // 3. UI Düzeni (Linear Studio Aesthetic — Dark Luxury Precision)
        ui.spacing_mut().item_spacing = Vec2::new(12.0, 10.0);

        // Header Panel (App Title + Status Pill)
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.heading(
                egui::RichText::new("STEINWAY & SONS")
                    .size(17.0)
                    .strong()
                    .color(Color32::from_rgb(240, 242, 245)),
            );
            ui.label(
                egui::RichText::new("Model D • Acoustic Grand")
                    .size(13.0)
                    .color(Color32::from_rgb(138, 143, 152)),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);

                // Pedal Rozeti
                let (pedal_bg, pedal_fg, pedal_txt) = if self.pedal_pressed {
                    (Color32::from_rgb(113, 112, 255), Color32::WHITE, "PEDAL: ENGAGED [SPACE]")
                } else {
                    (Color32::from_rgb(25, 26, 27), Color32::from_rgb(138, 143, 152), "PEDAL: OFF [SPACE]")
                };

                let pill = egui::Frame::new()
                    .fill(pedal_bg)
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(10, 4));
                pill.show(ui, |ui| {
                    ui.label(egui::RichText::new(pedal_txt).size(11.0).strong().color(pedal_fg));
                });

                ui.add_space(4.0);

                // Oktav Rozeti (Tab ile geçiş)
                let oct_pill = egui::Frame::new()
                    .fill(Color32::from_rgb(32, 34, 40))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(55, 58, 70)))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(10, 4));
                oct_pill.show(ui, |ui| {
                    let text = format!("OKTAV: {} & {} [TAB]", self.base_octave, self.base_octave + 1);
                    ui.label(egui::RichText::new(text).size(11.0).strong().color(Color32::from_rgb(215, 220, 235)));
                });

                ui.add_space(4.0);

                // Nota Modu Butonu (A-B-C vs Do-Re-Mi)
                let mode_label = if self.use_letter_notation {
                    "MOD: A-B-C-D-E-F-G"
                } else {
                    "MOD: Do-Re-Mi-Fa-Sol"
                };
                if ui.button(egui::RichText::new(mode_label).size(11.0).strong().color(Color32::from_rgb(113, 112, 255))).clicked() {
                    self.use_letter_notation = !self.use_letter_notation;
                }
            });
        });

        ui.add_space(2.0);

        // Studio Kontrol Kartı (Linear Elevated Surface #191a1b)
        let control_card = egui::Frame::new()
            .fill(Color32::from_rgb(25, 26, 27))
            .stroke(Stroke::new(1.0, Color32::from_rgb(35, 37, 42)))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(16, 12));

        control_card.show(ui, |ui| {
            ui.horizontal(|ui| {
                // Tuşe (Velocity) Slider
                ui.label(egui::RichText::new("Tuşe (Velocity):").size(13.0).color(Color32::from_rgb(208, 214, 224)));
                ui.add(egui::Slider::new(&mut self.velocity, 20..=127).text("↑/↓").show_value(true));
                ui.label(egui::RichText::new(format!("({})", velocity_label(self.velocity))).size(12.0).color(Color32::from_rgb(138, 143, 152)));

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(16.0);

                // Akustik Uzama Slider
                ui.label(egui::RichText::new("Çınlama Süresi:").size(13.0).color(Color32::from_rgb(208, 214, 224)));
                ui.add(egui::Slider::new(&mut self.min_sustain_ms, 100..=3000).text("ms [←/→]").show_value(true));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some((name, oct, midi)) = &self.last_played_info {
                        ui.label(
                            egui::RichText::new(format!("{} #{}", format!("{}{}", name, oct), midi))
                                .size(14.0)
                                .strong()
                                .color(Color32::from_rgb(113, 112, 255)),
                        );
                        ui.label(egui::RichText::new("Son Nota:").size(12.0).color(Color32::from_rgb(138, 143, 152)));
                    } else {
                        ui.label(egui::RichText::new("Canlı Dinlemede").size(12.0).color(Color32::from_rgb(98, 102, 109)));
                    }
                });
            });
        });

        ui.add_space(6.0);

        // Kesintisiz 2 Oktavlık Gerçek Piyano Gövdesi
        self.draw_continuous_piano(ui, shift_pressed, alt_pressed);

        ui.add_space(4.0);

        // Kısayol Bilgilendirme Çubuğu
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Oktav Değiştir: [TAB]  │  Bas Bölge: [Z..M]  │  Tiz Bölge: [Q..U]  │  Shift (+1) / Alt (+2)  │  Pedal: [Space]  │  Tuşe: [↑/↓]")
                    .size(11.5)
                    .color(Color32::from_rgb(110, 115, 125)),
            );
        });

        ctx.request_repaint();
    }
}

fn main() -> Result<(), eframe::Error> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Steinway & Sons Model D — Acoustic Grand Piano")
            .with_inner_size([960.0, 480.0])
            .with_min_inner_size([840.0, 440.0])
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "Acoustic Grand Piano",
        native_options,
        Box::new(|cc| {
            // Dark Mode Studio Paletini Uygula
            cc.egui_ctx.set_visuals(egui::Visuals {
                panel_fill: Color32::from_rgb(8, 9, 10),
                window_fill: Color32::from_rgb(8, 9, 10),
                override_text_color: Some(Color32::from_rgb(240, 242, 245)),
                ..egui::Visuals::dark()
            });
            Ok(Box::new(PianoApp::new(cc)))
        }),
    )
}
