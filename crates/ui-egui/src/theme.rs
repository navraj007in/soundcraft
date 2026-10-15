//! Design tokens. The palettes are the received UI design's (`docs/soundcraft-ai/design/tokens.json`,
//! `ui-design-spec.md`): the semantic tokens map one-to-one, and the component colours (buttons, chrome,
//! meter wells) are the midpoints of the design's gradients, which the custom-painted widgets draw in full.

use egui::{Color32, FontFamily, FontId};
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the Light palette is active. Dark is the default (the incumbent is dark-only).
static LIGHT_THEME: AtomicBool = AtomicBool::new(false);

/// Unit tests that change or read the process-wide palette must own it until their assertions
/// finish. The lock is re-entrant per thread, so a test may hold it and still build apps
/// (`SoundApp::new` takes it too) without deadlocking itself.
#[cfg(test)]
pub(crate) fn lock_test_palette() -> TestPaletteGuard {
    TestPaletteGuard::acquire()
}

/// RAII handle for the re-entrant, per-thread test palette lock; see [`lock_test_palette`].
#[cfg(test)]
pub(crate) struct TestPaletteGuard;

#[cfg(test)]
impl TestPaletteGuard {
    fn acquire() -> Self {
        palette_lock::acquire();
        Self
    }
}

#[cfg(test)]
impl Drop for TestPaletteGuard {
    fn drop(&mut self) {
        palette_lock::release();
    }
}

#[cfg(test)]
mod palette_lock {
    use std::cell::{Cell, RefCell};
    use std::sync::{Mutex, MutexGuard, PoisonError};

    static PALETTE: Mutex<()> = Mutex::new(());

    thread_local! {
        /// How many guards this thread holds; the OS lock is taken on 0→1 and released on 1→0.
        static DEPTH: Cell<usize> = const { Cell::new(0) };
        static GUARD: RefCell<Option<MutexGuard<'static, ()>>> = const { RefCell::new(None) };
    }

    pub(super) fn acquire() {
        DEPTH.with(|depth| {
            if depth.get() == 0 {
                GUARD.with(|guard| *guard.borrow_mut() = Some(PALETTE.lock().unwrap_or_else(PoisonError::into_inner)));
            }
            depth.set(depth.get() + 1);
        });
    }

    pub(super) fn release() {
        DEPTH.with(|depth| {
            let left = depth.get().saturating_sub(1);
            depth.set(left);
            if left == 0 {
                GUARD.with(|guard| *guard.borrow_mut() = None);
            }
        });
    }
}

/// The Appearance setting (`ui.theme {mode}`). Dark by default; System and Light are opt-in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    System,
    Light,
    #[default]
    Dark,
}

impl ThemeMode {
    pub const ALL: [Self; 3] = [Self::Dark, Self::Light, Self::System];

    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    /// The id used in `ui.theme {"mode": …}` and in the saved UI prefs.
    pub const fn id(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id().eq_ignore_ascii_case(s.trim()))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub window_bg: Color32,
    pub panel_bg: Color32,
    pub panel_bg2: Color32,
    pub toolbar_bg: Color32,
    pub toolbar_group: Color32,
    pub border: Color32,
    pub border_light: Color32,
    /// The lit top/left edge of raised panels: track headers and Mix strips.
    pub edge_light: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    /// Text on a filled control that is on or selected (dark teal fill in Dark, accent fill in Light).
    pub text_on_fill: Color32,
    /// Dark text for bright fills (a selected name, flags on a coloured chip).
    pub text_dark: Color32,
    pub header_text: Color32,
    pub counter_bg: Color32,
    pub counter_text: Color32,
    pub counter_label: Color32,
    pub accent: Color32,
    pub accent_dark: Color32,
    pub mode_on: Color32,
    pub mode_on_text: Color32,
    pub mode_off_text: Color32,
    pub button: Color32,
    pub button_hi: Color32,
    pub button_border: Color32,
    pub name_field: Color32,
    pub name_field_sel: Color32,
    pub ruler_bg: Color32,
    pub ruler_label_bg: Color32,
    pub ruler_text: Color32,
    pub ruler_tick: Color32,
    pub tempo_ruler: Color32,
    pub meter_ruler: Color32,
    pub marker_ruler: Color32,
    pub playlist_bg: Color32,
    pub playlist_alt: Color32,
    pub grid_line: Color32,
    pub bar_line: Color32,
    pub selection: Color32,
    pub playhead: Color32,
    pub insertion: Color32,
    pub solo: Color32,
    pub mute: Color32,
    pub rec: Color32,
    pub input: Color32,
    pub auto_read: Color32,
    pub auto_write: Color32,
    pub auto_touch: Color32,
    pub auto_latch: Color32,
    pub meter_green: Color32,
    pub meter_yellow: Color32,
    pub meter_red: Color32,
    pub fader_track: Color32,
    pub strip_bg: Color32,
    pub strip_section: Color32,
    pub slot_bg: Color32,
    pub automation_line: Color32,
    /// Menu bar background.
    pub menu_bar: Color32,
    /// Outline of toolbar groups.
    pub group_border: Color32,
    /// Edit-mode cell, not selected.
    pub mode_off: Color32,
    /// Counter-panel toggle (Grid, Count Off), off.
    pub counter_off: Color32,
    /// Outline of the counter panels.
    pub counter_border: Color32,
    /// Divider inside the counter panels.
    pub counter_rule: Color32,
    /// Smart-tool bracket, off.
    pub tool_bracket_off: Color32,
    /// Selected row in the Tracks, Groups and Clips lists.
    pub row_selected: Color32,
    /// Track-visible dot in the Tracks list.
    pub visible_dot: Color32,
    /// Big Counter background.
    pub big_counter_bg: Color32,
    /// Edit-window track header, selected.
    pub header_selected: Color32,
    /// Mix-window strip, selected.
    pub strip_selected: Color32,
    /// Marker names on the marker ruler.
    pub marker_text: Color32,
    /// Alternate-playlist lanes.
    pub playlist_lane: Color32,
    /// Universe overview background.
    pub universe_bg: Color32,
    /// Universe visible-area outline.
    pub universe_view: Color32,
    /// Meter background.
    pub meter_bg: Color32,
    /// Meter clip indicator, not clipped.
    pub meter_clip_off: Color32,
    /// Pan knob body.
    pub knob_bg: Color32,
    /// Pan knob outline.
    pub knob_ring: Color32,
    /// Pan knob pointer.
    pub knob_pointer: Color32,
    /// Mix-window section captions (INSERTS A-E, I/O…).
    pub section_label: Color32,
    /// Plugin slots, as Pro Tools draws them: the well they sit in, an empty slot, a used slot's
    /// face (top, bottom), a bypassed one's, and the dots, letters and arrows on them.
    pub slot_well: Color32,
    pub slot_empty: Color32,
    pub slot_face: [Color32; 2],
    pub slot_bypassed: [Color32; 2],
    pub slot_mark: Color32,
}

impl Tokens {
    pub const DARK: Tokens = Tokens {
        window_bg: Color32::from_rgb(27, 30, 34),
        panel_bg: Color32::from_rgb(35, 39, 44),
        panel_bg2: Color32::from_rgb(46, 51, 56),
        toolbar_bg: Color32::from_rgb(45, 50, 56),
        toolbar_group: Color32::from_rgb(35, 39, 44),
        border: Color32::from_rgb(14, 16, 18),
        border_light: Color32::from_rgb(58, 64, 71),
        text: Color32::from_rgb(230, 233, 237),
        text_dim: Color32::from_rgb(169, 177, 186),
        text_on_fill: Color32::from_rgb(214, 251, 246),
        text_dark: Color32::from_rgb(16, 16, 16),
        header_text: Color32::from_rgb(230, 233, 237),
        counter_bg: Color32::from_rgb(10, 12, 14),
        counter_text: Color32::from_rgb(159, 243, 232),
        counter_label: Color32::from_rgb(169, 177, 186),
        accent: Color32::from_rgb(62, 199, 184),
        accent_dark: Color32::from_rgb(47, 154, 141),
        mode_on: Color32::from_rgb(42, 99, 92),
        mode_on_text: Color32::from_rgb(214, 251, 246),
        mode_off_text: Color32::from_rgb(169, 177, 186),
        button: Color32::from_rgb(61, 66, 74),
        button_hi: Color32::from_rgb(72, 79, 88),
        button_border: Color32::from_rgb(10, 12, 14),
        name_field: Color32::from_rgb(46, 51, 56),
        name_field_sel: Color32::from_rgb(58, 64, 71),
        ruler_bg: Color32::from_rgb(46, 51, 56),
        ruler_label_bg: Color32::from_rgb(46, 51, 56),
        ruler_text: Color32::from_rgb(169, 177, 186),
        ruler_tick: Color32::from_rgb(107, 118, 131),
        tempo_ruler: Color32::from_rgb(43, 106, 102),
        meter_ruler: Color32::from_rgb(63, 91, 133),
        marker_ruler: Color32::from_rgb(46, 51, 56),
        playlist_bg: Color32::from_rgb(27, 30, 34),
        playlist_alt: Color32::from_rgb(31, 34, 39),
        grid_line: Color32::from_rgb(41, 45, 51),
        bar_line: Color32::from_rgb(53, 59, 65),
        selection: Color32::from_rgba_premultiplied(17, 56, 52, 71),
        playhead: Color32::from_rgb(255, 93, 93),
        insertion: Color32::from_rgb(230, 233, 237),
        solo: Color32::from_rgb(242, 193, 78),
        mute: Color32::from_rgb(91, 155, 255),
        rec: Color32::from_rgb(255, 93, 93),
        input: Color32::from_rgb(88, 214, 125),
        auto_read: Color32::from_rgb(88, 214, 125),
        auto_write: Color32::from_rgb(255, 93, 93),
        auto_touch: Color32::from_rgb(242, 193, 78),
        auto_latch: Color32::from_rgb(242, 193, 78),
        meter_green: Color32::from_rgb(62, 199, 184),
        meter_yellow: Color32::from_rgb(242, 193, 78),
        meter_red: Color32::from_rgb(255, 93, 93),
        fader_track: Color32::from_rgb(16, 18, 20),
        strip_bg: Color32::from_rgb(46, 51, 56),
        strip_section: Color32::from_rgb(35, 39, 44),
        slot_bg: Color32::from_rgb(10, 12, 14),
        automation_line: Color32::from_rgb(230, 233, 237),
        menu_bar: Color32::from_rgb(27, 30, 34),
        group_border: Color32::from_rgb(14, 16, 18),
        mode_off: Color32::from_rgb(61, 66, 74),
        counter_off: Color32::from_rgb(46, 51, 56),
        counter_border: Color32::from_rgb(14, 16, 18),
        counter_rule: Color32::from_rgb(58, 64, 71),
        tool_bracket_off: Color32::from_rgb(58, 64, 71),
        row_selected: Color32::from_rgb(58, 64, 71),
        visible_dot: Color32::from_rgb(169, 177, 186),
        big_counter_bg: Color32::from_rgb(10, 12, 14),
        header_selected: Color32::from_rgb(46, 51, 56),
        strip_selected: Color32::from_rgb(46, 51, 56),
        marker_text: Color32::from_rgb(230, 233, 237),
        playlist_lane: Color32::from_rgb(31, 34, 39),
        universe_bg: Color32::from_rgb(10, 12, 14),
        universe_view: Color32::from_rgb(230, 233, 237),
        meter_bg: Color32::from_rgb(16, 18, 20),
        meter_clip_off: Color32::from_rgb(58, 31, 33),
        knob_bg: Color32::from_rgb(46, 51, 56),
        knob_ring: Color32::from_rgb(107, 118, 131),
        knob_pointer: Color32::from_rgb(230, 233, 237),
        section_label: Color32::from_rgb(169, 177, 186),
        edge_light: Color32::from_rgba_premultiplied(20, 20, 20, 20),
        slot_well: Color32::from_rgb(43, 43, 43),
        slot_empty: Color32::from_rgb(39, 39, 39),
        slot_face: [Color32::from_rgb(92, 92, 92), Color32::from_rgb(75, 75, 75)],
        slot_bypassed: [Color32::from_rgb(66, 96, 146), Color32::from_rgb(50, 76, 120)],
        slot_mark: Color32::from_rgb(159, 159, 159),
    };

    pub const LIGHT: Tokens = Tokens {
        window_bg: Color32::from_rgb(228, 232, 236),
        panel_bg: Color32::from_rgb(242, 244, 246),
        panel_bg2: Color32::from_rgb(255, 255, 255),
        toolbar_bg: Color32::from_rgb(237, 240, 243),
        toolbar_group: Color32::from_rgb(242, 244, 246),
        border: Color32::from_rgb(201, 208, 215),
        border_light: Color32::from_rgb(221, 226, 231),
        text: Color32::from_rgb(20, 24, 28),
        text_dim: Color32::from_rgb(71, 82, 93),
        text_on_fill: Color32::from_rgb(255, 255, 255),
        text_dark: Color32::from_rgb(16, 17, 20),
        header_text: Color32::from_rgb(20, 24, 28),
        counter_bg: Color32::from_rgb(255, 255, 255),
        counter_text: Color32::from_rgb(8, 86, 79),
        counter_label: Color32::from_rgb(71, 82, 93),
        accent: Color32::from_rgb(11, 111, 102),
        accent_dark: Color32::from_rgb(8, 86, 79),
        mode_on: Color32::from_rgb(11, 111, 102),
        mode_on_text: Color32::from_rgb(255, 255, 255),
        mode_off_text: Color32::from_rgb(71, 82, 93),
        button: Color32::from_rgb(242, 244, 246),
        button_hi: Color32::from_rgb(248, 249, 250),
        button_border: Color32::from_rgb(185, 193, 201),
        name_field: Color32::from_rgb(255, 255, 255),
        name_field_sel: Color32::from_rgb(221, 226, 231),
        ruler_bg: Color32::from_rgb(255, 255, 255),
        ruler_label_bg: Color32::from_rgb(255, 255, 255),
        ruler_text: Color32::from_rgb(71, 82, 93),
        ruler_tick: Color32::from_rgb(118, 129, 141),
        tempo_ruler: Color32::from_rgb(130, 178, 176),
        meter_ruler: Color32::from_rgb(139, 170, 216),
        marker_ruler: Color32::from_rgb(255, 255, 255),
        playlist_bg: Color32::from_rgb(228, 232, 236),
        playlist_alt: Color32::from_rgb(235, 238, 241),
        grid_line: Color32::from_rgb(225, 229, 234),
        bar_line: Color32::from_rgb(222, 227, 232),
        selection: Color32::from_rgba_premultiplied(2, 22, 20, 51),
        playhead: Color32::from_rgb(192, 38, 45),
        insertion: Color32::from_rgb(20, 24, 28),
        solo: Color32::from_rgb(133, 84, 0),
        mute: Color32::from_rgb(31, 95, 191),
        rec: Color32::from_rgb(192, 38, 45),
        input: Color32::from_rgb(22, 112, 58),
        auto_read: Color32::from_rgb(22, 112, 58),
        auto_write: Color32::from_rgb(192, 38, 45),
        auto_touch: Color32::from_rgb(133, 84, 0),
        auto_latch: Color32::from_rgb(133, 84, 0),
        meter_green: Color32::from_rgb(11, 122, 112),
        meter_yellow: Color32::from_rgb(217, 154, 0),
        meter_red: Color32::from_rgb(192, 38, 45),
        fader_track: Color32::from_rgb(210, 216, 222),
        strip_bg: Color32::from_rgb(255, 255, 255),
        strip_section: Color32::from_rgb(242, 244, 246),
        slot_bg: Color32::from_rgb(255, 255, 255),
        automation_line: Color32::from_rgb(20, 24, 28),
        menu_bar: Color32::from_rgb(228, 232, 236),
        group_border: Color32::from_rgb(201, 208, 215),
        mode_off: Color32::from_rgb(242, 244, 246),
        counter_off: Color32::from_rgb(255, 255, 255),
        counter_border: Color32::from_rgb(201, 208, 215),
        counter_rule: Color32::from_rgb(221, 226, 231),
        tool_bracket_off: Color32::from_rgb(221, 226, 231),
        row_selected: Color32::from_rgb(221, 226, 231),
        visible_dot: Color32::from_rgb(71, 82, 93),
        big_counter_bg: Color32::from_rgb(255, 255, 255),
        header_selected: Color32::from_rgb(255, 255, 255),
        strip_selected: Color32::from_rgb(255, 255, 255),
        marker_text: Color32::from_rgb(20, 24, 28),
        playlist_lane: Color32::from_rgb(235, 238, 241),
        universe_bg: Color32::from_rgb(255, 255, 255),
        universe_view: Color32::from_rgb(20, 24, 28),
        meter_bg: Color32::from_rgb(210, 216, 222),
        meter_clip_off: Color32::from_rgb(232, 201, 203),
        knob_bg: Color32::from_rgb(255, 255, 255),
        knob_ring: Color32::from_rgb(118, 129, 141),
        knob_pointer: Color32::from_rgb(20, 24, 28),
        section_label: Color32::from_rgb(71, 82, 93),
        edge_light: Color32::from_rgba_premultiplied(150, 150, 150, 150),
        slot_well: Color32::from_rgb(212, 214, 218),
        slot_empty: Color32::from_rgb(198, 200, 205),
        slot_face: [Color32::from_rgb(244, 245, 247), Color32::from_rgb(214, 216, 220)],
        slot_bypassed: [Color32::from_rgb(190, 210, 240), Color32::from_rgb(160, 186, 228)],
        slot_mark: Color32::from_rgb(92, 95, 102),
    };

    pub fn current() -> Tokens {
        if is_light() { Self::LIGHT } else { Self::DARK }
    }
}

/// Whether the Light palette is the active one.
pub fn is_light() -> bool {
    LIGHT_THEME.load(Ordering::Relaxed)
}

/// Resolve each frame from the integration's current OS appearance. Unknown uses Dark.
pub fn wants_light(ctx: &egui::Context, mode: ThemeMode) -> bool {
    match mode {
        ThemeMode::Light => true,
        ThemeMode::Dark => false,
        ThemeMode::System => ctx.system_theme() == Some(egui::Theme::Light),
    }
}

/// A track's header and lane backgrounds in the Edit window, tinted with its colour by `tint` (0..1).
pub fn track_backgrounds(color: [u8; 3], selected: bool, tint: f32) -> (Color32, Color32) {
    let t = Tokens::current();
    let k = tint.clamp(0.0, 1.0);
    if k <= 0.0 {
        return if selected { (t.header_selected, t.playlist_alt) } else { (t.panel_bg2, t.playlist_bg) };
    }
    let c = rgb(color);
    let header = t.panel_bg2.lerp_to_gamma(c, k * 0.55);
    let header = if selected { crate::widgets::shade(header, 0.1) } else { header };
    let lane = if selected { t.playlist_alt } else { t.playlist_bg }.lerp_to_gamma(c, k * 0.2);
    (header, lane)
}

/// The four colours of a clip for a track colour, from the design's OKLCH recipe (see `oklch.rs`).
pub fn clip_roles(rgb: [u8; 3]) -> crate::oklch::ClipRoles {
    crate::oklch::clip_roles_for(rgb, is_light())
}

/// Clip body, name bar and waveform colours for a track colour. A selected clip is drawn with the
/// design's 2 px accent outline (see `draw_clip`), not a different fill.
pub fn clip_colors(rgb: [u8; 3], _selected: bool) -> (Color32, Color32, Color32) {
    let r = clip_roles(rgb);
    (r.body, r.header, r.waveform)
}

pub fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Plex Sans SemiBold (600).
pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(crate::fonts::BOLD.into()))
}

/// Plex Sans Medium (500).
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(crate::fonts::MEDIUM.into()))
}

/// Plex Mono Medium (500), for the bars counter.
pub fn mono_medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(crate::fonts::MONO_MEDIUM.into()))
}

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// Make `light` (or dark) the active palette and apply its visuals. Call only when it changes.
pub fn apply(ctx: &egui::Context, light: bool) {
    LIGHT_THEME.store(light, Ordering::Relaxed);
    let t = Tokens::current();
    let mut v = if light { egui::Visuals::light() } else { egui::Visuals::dark() };
    let control = egui::CornerRadius::same(4);
    let line = egui::Stroke::new(1.0, t.border);
    v.panel_fill = t.panel_bg;
    v.window_fill = t.panel_bg2;
    v.extreme_bg_color = t.slot_bg;
    v.faint_bg_color = t.panel_bg2;
    v.code_bg_color = t.slot_bg;
    v.hyperlink_color = t.accent;
    // Text selection and the focus ring: an accent tint with a 2 px accent outline (design: focus ring 2 + 2 offset).
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(t.accent.r(), t.accent.g(), t.accent.b(), if light { 56 } else { 80 });
    v.selection.stroke = egui::Stroke::new(2.0, t.accent);
    // Controls: 4 px radius, 1 px borders; the design's offset shadow has no blur.
    v.widgets.noninteractive.bg_fill = t.panel_bg;
    v.widgets.noninteractive.weak_bg_fill = t.panel_bg;
    v.widgets.noninteractive.bg_stroke = line;
    v.widgets.noninteractive.fg_stroke.color = t.text;
    v.widgets.inactive.weak_bg_fill = t.button;
    v.widgets.inactive.bg_fill = t.button;
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, t.button_border);
    v.widgets.inactive.fg_stroke.color = t.text;
    v.widgets.hovered.weak_bg_fill = t.button_hi;
    v.widgets.hovered.bg_fill = t.button_hi;
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, t.button_border);
    v.widgets.hovered.fg_stroke.color = t.text;
    v.widgets.active.weak_bg_fill = t.mode_on;
    v.widgets.active.bg_fill = t.mode_on;
    v.widgets.active.bg_stroke = egui::Stroke::new(1.0, t.accent);
    v.widgets.active.fg_stroke.color = t.text_on_fill;
    v.widgets.open.weak_bg_fill = t.button_hi;
    v.widgets.open.bg_fill = t.button_hi;
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = control;
    }
    v.window_corner_radius = egui::CornerRadius::same(6);
    v.menu_corner_radius = egui::CornerRadius::same(4);
    v.window_stroke = egui::Stroke::new(1.0, t.border_light);
    let shadow = egui::epaint::Shadow { offset: [2, 2], blur: 0, spread: 0, color: Color32::from_black_alpha(if light { 56 } else { 90 }) };
    v.window_shadow = shadow;
    v.popup_shadow = shadow;
    // Keep receiving native OS appearance changes even when our palette is manual. On macOS,
    // pinning egui's theme also pins the window appearance and hides later OS changes.
    ctx.set_theme(egui::ThemePreference::System);
    // Both egui style branches must use our resolved palette: egui can select either branch
    // as the OS changes, including while SoundCraft's manual Light/Dark choice stays fixed.
    ctx.all_styles_mut(|s| {
        s.visuals = v.clone();
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(6.0, 2.0);
        s.interaction.tooltip_delay = 0.4;
        // The design's type scale: body 13/20, label 12, caption 11, panel heading 14 SemiBold, numbers Plex Mono 12.
        use egui::TextStyle as T;
        s.text_styles =
            [(T::Heading, bold(14.0)), (T::Body, regular(13.0)), (T::Button, regular(13.0)), (T::Small, regular(11.0)), (T::Monospace, mono(12.0))]
                .into();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Services, SoundApp, UiState};
    use serde_json::json;

    fn lum(c: Color32) -> f32 {
        let lin = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    fn hex(c: Color32) -> String {
        format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
    }

    /// The semantic tokens are the design's, value for value (`design/tokens.json`).
    #[test]
    fn the_palettes_carry_the_designs_semantic_tokens() {
        let design: serde_json::Value = serde_json::from_str(include_str!("../../../docs/soundcraft-ai/design/tokens.json")).unwrap();
        for (theme, palette) in [("dark", Tokens::DARK), ("light", Tokens::LIGHT)] {
            let get = |name: &str| -> String {
                design["themes"][theme]["colors"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|c| c["name"] == name)
                    .unwrap_or_else(|| panic!("no design token {name}"))["value"]
                    .as_str()
                    .unwrap()
                    .to_string()
            };
            for (token, field) in [
                ("bg.canvas", palette.window_bg),
                ("bg.surface.1", palette.panel_bg),
                ("bg.surface.2", palette.panel_bg2),
                ("bg.surface.3", palette.border_light),
                ("bg.inset", palette.counter_bg),
                ("border.subtle", palette.border),
                ("text.primary", palette.text),
                ("text.secondary", palette.text_dim),
                ("accent.default", palette.accent),
                ("transport.record", palette.rec),
                ("track.solo", palette.solo),
                ("track.mute", palette.mute),
            ] {
                assert_eq!(hex(field), get(token), "{theme}: {token}");
            }
        }
    }

    /// The design's minimums: text 4.5:1, accent, status and graphics 3:1, on the surface they sit on.
    #[test]
    fn the_palettes_meet_the_designs_contrast_minimums() {
        for (theme, t) in [("dark", Tokens::DARK), ("light", Tokens::LIGHT)] {
            let mut failures = Vec::new();
            let mut need = |name: &str, fg: Color32, bg: Color32, min: f32| {
                let c = contrast(fg, bg);
                if c < min {
                    failures.push(format!("{name}: {c:.2}:1 is under {min}:1"));
                }
            };
            for (surface_name, surface) in [("canvas", t.window_bg), ("panel", t.panel_bg), ("raised", t.panel_bg2)] {
                need(&format!("text on {surface_name}"), t.text, surface, 4.5);
                need(&format!("dim text on {surface_name}"), t.text_dim, surface, 4.5);
                for (n, c) in [("accent", t.accent), ("solo", t.solo), ("mute", t.mute), ("record", t.rec), ("input", t.input)] {
                    need(&format!("{n} on {surface_name}"), c, surface, 3.0);
                }
            }
            need("text on buttons", t.text, t.button, 4.5);
            need("text on raised buttons", t.text, t.button_hi, 4.5);
            need("text on a selected control", t.mode_on_text, t.mode_on, 4.5);
            need("counter", t.counter_text, t.counter_bg, 4.5);
            need("ruler text", t.ruler_text, t.ruler_bg, 4.5);
            need("meter fill on its well", t.meter_green, t.meter_bg, 3.0);
            assert!(failures.is_empty(), "{theme}: {failures:#?}");
        }
    }

    #[test]
    fn test_palette_lock_is_reentrant_and_excludes_other_threads() {
        // Regression for the parallel-run flakes: the process-wide palette must be owned for a
        // whole test, re-entrant on the owning thread (so `SoundApp::new` can take it too), and
        // exclusive against other threads (so a frame cannot flip it mid-test).
        let outer = lock_test_palette();
        let inner = lock_test_palette();
        let app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _held = lock_test_palette();
            let _ = tx.send(());
        });
        assert!(rx.recv_timeout(std::time::Duration::from_millis(100)).is_err(), "another thread must wait for the palette lock");
        drop(inner);
        drop(outer);
        drop(app);
        // Other tests compete for the lock too, and some hold it for seconds: allow for a queue.
        assert!(rx.recv_timeout(std::time::Duration::from_secs(120)).is_ok(), "the palette lock must be released when every holder drops it");
    }

    #[test]
    fn dark_is_the_default() {
        assert_eq!(ThemeMode::default(), ThemeMode::Dark);
        assert_eq!(UiState::default().theme, ThemeMode::Dark);
        // Prefs saved before the setting existed load as Dark.
        let old: UiState = serde_json::from_value(json!({"show_tracks_list": true})).unwrap_or_default();
        assert_eq!(old.theme, ThemeMode::Dark);
    }

    #[test]
    fn prefs_saved_before_track_tint_get_the_default() {
        let ui: UiState = serde_json::from_value(json!({"theme": "light"})).unwrap();
        assert!((ui.track_tint - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn modes_round_trip_through_prefs() {
        for mode in ThemeMode::ALL {
            assert_eq!(ThemeMode::parse(mode.id()), Some(mode));
            let ui = UiState { theme: mode, ..UiState::default() };
            let back: UiState = serde_json::from_value(serde_json::to_value(&ui).unwrap_or_default()).unwrap_or_default();
            assert_eq!(back.theme, mode);
        }
        assert_eq!(ThemeMode::parse(" Light "), Some(ThemeMode::Light));
        assert_eq!(ThemeMode::parse("sepia"), None);
    }

    #[test]
    fn theme_command_sets_and_reports_the_mode() {
        let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
        assert_eq!(app.run("ui.theme", json!({})), Ok(json!({"mode": "dark"})));
        assert_eq!(app.run("ui.theme", json!({"mode": "light"})), Ok(json!({"mode": "light"})));
        assert_eq!(app.ui.theme, ThemeMode::Light);
        assert!(app.run("ui.theme", json!({"mode": "sepia"})).is_err());
        assert_eq!(app.ui.theme, ThemeMode::Light);
        assert_eq!(app.run("ui.theme", json!({"mode": "system"})), Ok(json!({"mode": "system"})));
        assert!(crate::menus::UI_COMMANDS.iter().any(|c| c.0 == "ui.theme"));
    }

    #[test]
    fn application_frames_follow_system_appearance_and_preserve_manual_palettes() {
        // Mixer rendering tests also change the process-wide custom-painted palette.
        let _palette = lock_test_palette();
        fn frame(app: &mut SoundApp, ctx: &egui::Context, system_theme: Option<egui::Theme>, light: bool) {
            let mode = app.ui.theme;
            let colors = |app: &SoundApp| {
                app.engine
                    .session()
                    .tracks
                    .iter()
                    .map(|track| {
                        (track.color, track.playlists.iter().flat_map(|playlist| &playlist.clips).map(|clip| clip.color).collect::<Vec<_>>())
                    })
                    .collect::<Vec<_>>()
            };
            let before = colors(app);
            let mut output = ctx.run_ui(egui::RawInput { system_theme, ..Default::default() }, |ui| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                let tokens = if light { Tokens::LIGHT } else { Tokens::DARK };
                assert_eq!(is_light(), light, "custom palette for {mode:?} with {system_theme:?}");
                assert_eq!(Tokens::current().panel_bg, tokens.panel_bg);
                assert_eq!(ctx.options(|options| options.theme_preference), egui::ThemePreference::System);
                for theme in [egui::Theme::Light, egui::Theme::Dark] {
                    let style = ctx.style_of(theme);
                    assert_eq!(style.visuals.dark_mode, !light);
                    assert_eq!(style.visuals.panel_fill, tokens.panel_bg);
                    assert_eq!(style.visuals.widgets.inactive.bg_fill, tokens.button);
                    assert_eq!(style.visuals.widgets.inactive.fg_stroke.color, tokens.text);
                    assert_eq!(style.spacing.button_padding, egui::vec2(6.0, 2.0));
                }
                app.ui(ui);
                assert_eq!(ui.visuals().panel_fill, tokens.panel_bg);
                assert_eq!(ui.visuals().widgets.inactive.bg_fill, tokens.button);
            });
            output.textures_delta.clear();
            assert_eq!(app.ui.theme, mode);
            assert_eq!(serde_json::to_value(&app.ui).unwrap()["theme"], mode.id());
            assert_eq!(colors(app), before, "appearance must preserve track and clip colors");
        }

        let appearances = [Some(egui::Theme::Light), Some(egui::Theme::Dark), Some(egui::Theme::Light), None];
        for initial in [Some(egui::Theme::Light), Some(egui::Theme::Dark), None] {
            let ctx = egui::Context::default();
            let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, Services::default());
            app.ui = serde_json::from_value(json!({"theme": "system"})).unwrap();
            frame(&mut app, &ctx, initial, initial == Some(egui::Theme::Light));
        }

        let ctx = egui::Context::default();
        let mut engine = soundcraft_engine::demo::demo_engine();
        let track = engine.session_mut().tracks.first_mut().unwrap();
        track.color = [19, 83, 147];
        track.playlists.iter_mut().flat_map(|playlist| &mut playlist.clips).next().unwrap().color = Some([211, 57, 99]);
        let mut app = SoundApp::new(engine, None, Services::default());
        for mode in [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark] {
            app.run("ui.theme", json!({"mode": mode.id()})).unwrap();
            for appearance in appearances {
                let light = mode == ThemeMode::Light || (mode == ThemeMode::System && appearance == Some(egui::Theme::Light));
                frame(&mut app, &ctx, appearance, light);
            }
            if mode != ThemeMode::System {
                let latest = if mode == ThemeMode::Light { egui::Theme::Dark } else { egui::Theme::Light };
                frame(&mut app, &ctx, Some(latest), mode == ThemeMode::Light);
                app.run("ui.theme", json!({"mode": "system"})).unwrap();
                // Resolve the current context immediately, without another OS appearance event.
                app.logic(&ctx);
                assert_eq!(is_light(), latest == egui::Theme::Light);
                assert_eq!(ctx.global_style().visuals.dark_mode, latest != egui::Theme::Light);
                frame(&mut app, &ctx, Some(latest), latest == egui::Theme::Light);
            }
        }
    }
}
