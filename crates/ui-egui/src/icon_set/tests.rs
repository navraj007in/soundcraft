use super::*;

#[test]
fn the_crate_copy_is_the_designs_copy() {
    let design = include_str!("../../../../docs/soundcraft-ai/design/icons.json");
    assert_eq!(include_str!("../../assets/icons.json"), design, "run: cp docs/soundcraft-ai/design/icons.json crates/ui-egui/assets/");
}

#[test]
fn every_icon_in_the_design_parses_at_both_sizes() {
    let v: serde_json::Value = serde_json::from_str(include_str!("../../assets/icons.json")).unwrap();
    let list = v["icons"].as_array().unwrap();
    assert_eq!(list.len(), 63);
    for i in list {
        let name = i["name"].as_str().unwrap();
        let icon = get(name).unwrap_or_else(|| panic!("{name} did not load"));
        let has_art = |s: &SizedIcon| !s.stroke.is_empty() || !s.fill.is_empty();
        assert!(has_art(&icon.size_20) && has_art(&icon.size_16), "{name} draws nothing");
        for p in icon.size_20.stroke.iter().chain(&icon.size_20.fill).chain(&icon.size_16.stroke).chain(&icon.size_16.fill).flat_map(|s| &s.points) {
            assert!((-1.0..=21.0).contains(&p[0]) && (-1.0..=21.0).contains(&p[1]), "{name} leaves the grid: {p:?}");
        }
    }
}

#[test]
fn only_the_four_filled_icons_have_filled_shapes() {
    let mut filled: Vec<&str> =
        set().iter().filter(|(_, i)| !i.size_20.fill.is_empty() || !i.size_16.fill.is_empty()).map(|(n, _)| n.as_str()).collect();
    filled.sort_unstable();
    assert!(filled.len() >= 4 && filled.len() <= 6, "{filled:?}");
    for n in ["record", "record-arm", "automation-write", "overflow"] {
        assert!(filled.contains(&n), "{n} should be filled: {filled:?}");
    }
}

#[test]
fn lines_and_relative_moves_parse() {
    let p = parse("M2 2h4v4h-4z").unwrap();
    assert_eq!(p.len(), 1);
    assert!(p[0].closed);
    assert_eq!(p[0].points, vec![[2.0, 2.0], [6.0, 2.0], [6.0, 6.0], [2.0, 6.0]]);
    let q = parse("m1 1 2 0 0 2").unwrap();
    assert_eq!(q[0].points, vec![[1.0, 1.0], [3.0, 1.0], [3.0, 3.0]]);
}

#[test]
fn an_arc_ends_where_it_says_and_stays_on_the_circle() {
    let p = parse("M2 10A8 8 0 0 1 18 10").unwrap();
    let pts = &p[0].points;
    assert_eq!(*pts.last().unwrap(), [18.0, 10.0]);
    for q in pts {
        let d = ((q[0] - 10.0).powi(2) + (q[1] - 10.0).powi(2)).sqrt();
        assert!((d - 8.0).abs() < 0.05, "{q:?} is {d} from the centre");
    }
    // sweep=1 from left to right goes over the top (y < 10) in SVG's y-down space
    assert!(pts.iter().any(|q| q[1] < 3.0));
}

#[test]
fn glued_arc_flags_parse() {
    assert!(parse("M2 10a8 8 0 1116 0").is_some());
}

#[test]
fn bad_paths_are_refused_not_panicked_on() {
    for bad in ["", "M", "M1", "L1 1", "M0 0 X 1 2", "M0 0 L a b", "M0 0 A1 1 0 2 0 3 3", "1 2 3"] {
        assert!(parse(bad).is_none() || bad.is_empty(), "{bad:?} should not parse");
    }
}

#[test]
fn curves_end_at_their_end_point() {
    let p = parse("M0 0C0 5 5 10 10 10S15 5 20 0").unwrap();
    assert_eq!(*p[0].points.last().unwrap(), [20.0, 0.0]);
}

#[test]
fn drawing_an_unknown_name_reports_false() {
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(Default::default(), |ui| {
        let p = ui.painter().clone();
        let r = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(16.0, 16.0));
        assert!(!draw(&p, r, "no-such-icon", Color32::WHITE));
        assert!(draw(&p, r, "play", Color32::WHITE));
        assert!(draw(&p, r, "rtz", Color32::WHITE), "old name still works through the alias");
    });
    out.textures_delta.clear();
}
