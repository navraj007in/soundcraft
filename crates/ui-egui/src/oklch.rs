//! Clip colours from a track's hue, as the UI design defines them (`docs/soundcraft-ai/design/tokens.json`,
//! `clip_palette`): each of four roles (body, header, waveform, border) is a fixed lightness and chroma
//! in the OKLCH colour space at the track's hue, so any track colour gets a coherent, legible clip.

use egui::Color32;

/// Linear sRGB from a gamma-encoded 0 to 255 channel.
fn linear(c: u8) -> f32 {
    let v = f32::from(c) / 255.0;
    if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn encode(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let g = if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (g * 255.0).round().clamp(0.0, 255.0) as u8
}

/// OKLCH of an sRGB colour: lightness 0 to 1, chroma (about 0 to 0.4) and hue in degrees.
pub fn from_rgb(rgb: [u8; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (linear(rgb[0]), linear(rgb[1]), linear(rgb[2]));
    let l = (0.412_221_47 * r + 0.536_332_54 * g + 0.051_445_99 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_84 * g + 0.629_978_7 * b).cbrt();
    let (lab_l, a, bb) = (
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    );
    let h = bb.atan2(a).to_degrees();
    (lab_l, a.hypot(bb), if h < 0.0 { h + 360.0 } else { h })
}

/// The sRGB colour for an OKLCH value, clamped into gamut.
pub fn to_color(l: f32, c: f32, h_deg: f32) -> Color32 {
    let (a, b) = (c * h_deg.to_radians().cos(), c * h_deg.to_radians().sin());
    let (l_, m_, s_) = (
        (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3),
        (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3),
        (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3),
    );
    let r = 4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_93 * s_;
    let g = -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_4 * s_;
    let bl = -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_;
    Color32::from_rgb(encode(r), encode(g), encode(bl))
}

/// The four colours of a clip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipRoles {
    pub body: Color32,
    pub header: Color32,
    pub waveform: Color32,
    pub border: Color32,
}

/// The design's recipe: (lightness, chroma) per role, for the dark and the light theme.
pub fn clip_roles(hue: f32, chroma_scale: f32, light: bool) -> ClipRoles {
    let k = chroma_scale.clamp(0.0, 1.0);
    let c = |l: f32, chroma: f32| to_color(l, chroma * k, hue);
    if light {
        ClipRoles { body: c(0.88, 0.07), header: c(0.80, 0.10), waveform: c(0.36, 0.10), border: c(0.62, 0.10) }
    } else {
        ClipRoles { body: c(0.35, 0.07), header: c(0.46, 0.10), waveform: c(0.84, 0.10), border: c(0.55, 0.10) }
    }
}

/// Clip roles for an arbitrary track colour: its hue, and a neutral clip for a grey track.
pub fn clip_roles_for(rgb: [u8; 3], light: bool) -> ClipRoles {
    let (_, chroma, hue) = from_rgb(rgb);
    // Below about 0.03 the hue is noise; fade chroma out so a grey track gets a grey clip.
    clip_roles(hue, (chroma / 0.06).clamp(0.0, 1.0), light)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> [u8; 3] {
        let n = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).unwrap();
        [n(1), n(3), n(5)]
    }

    fn close(c: Color32, h: &str) -> bool {
        let w = hex(h);
        [c.r(), c.g(), c.b()].iter().zip(w).all(|(a, b)| a.abs_diff(b) <= 1)
    }

    /// All 12 hues x 8 colours of the design's clip palette, from the committed design data.
    #[test]
    fn reproduces_the_designs_clip_palette() {
        let tokens: serde_json::Value = serde_json::from_str(include_str!("../../../docs/soundcraft-ai/design/tokens.json")).unwrap();
        let palette = tokens["clip_palette"].as_array().unwrap();
        assert_eq!(palette.len(), 12);
        for p in palette {
            let hue = p["hue"].as_f64().unwrap() as f32;
            for (light, key) in [(false, "dark"), (true, "light")] {
                let roles = clip_roles(hue, 1.0, light);
                for (role, got) in [("body", roles.body), ("header", roles.header), ("waveform", roles.waveform), ("border", roles.border)] {
                    let want = p[key][role].as_str().unwrap();
                    assert!(close(got, want), "{} {key} {role}: got {got:?}, the design says {want}", p["name"]);
                }
            }
        }
    }

    #[test]
    fn rgb_round_trips_through_oklch() {
        for rgb in [[200u8, 80, 60], [40, 160, 220], [120, 120, 120], [250, 250, 250], [10, 10, 10]] {
            let (l, c, h) = from_rgb(rgb);
            let back = to_color(l, c, h);
            assert!([back.r(), back.g(), back.b()].iter().zip(rgb).all(|(a, b)| a.abs_diff(b) <= 1), "{rgb:?} -> {back:?}");
        }
    }

    #[test]
    fn a_grey_track_gets_a_grey_clip_and_a_coloured_one_keeps_its_hue() {
        let grey = clip_roles_for([128, 128, 128], false);
        for c in [grey.body, grey.header, grey.waveform, grey.border] {
            assert!(c.r().abs_diff(c.g()) <= 2 && c.g().abs_diff(c.b()) <= 2, "{c:?} is not grey");
        }
        let (_, _, hue) = from_rgb([200, 60, 60]);
        let (_, _, clip_hue) = from_rgb({
            let r = clip_roles_for([200, 60, 60], false).header;
            [r.r(), r.g(), r.b()]
        });
        assert!((hue - clip_hue).abs() < 6.0, "{hue} vs {clip_hue}");
    }

    #[test]
    fn waveforms_stay_legible_on_their_bodies() {
        // The design checks waveform against body at 3:1 or better in both themes, for its 12 hues.
        let lum = |c: Color32| 0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b());
        for hue in (0..360).step_by(15) {
            for light in [false, true] {
                let r = clip_roles(hue as f32, 1.0, light);
                let (a, b) = (lum(r.waveform), lum(r.body));
                assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 3.0, "hue {hue} light={light}");
            }
        }
    }
}
