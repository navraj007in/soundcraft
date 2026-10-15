//! Which MIDI output port each MIDI track plays to, and which ports pass the keyboard through.

use super::*;
use crate::cmd;
use serde_json::json;
use soundcraft_model::{Route, TrackKind};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "midi.track_output",
            "MIDI Track Output",
            [],
            None,
            "{tracks?, port: name | null} — sends the MIDI tracks' notes and controllers to that MIDI output port (null: back to no port, so a hosted instrument plays them)",
            has_selection,
            track_output
        ),
        cmd!(
            noundo "setup.midi_thru",
            "MIDI Thru",
            ["Setup", "MIDI"],
            None,
            "{enable?: [port names], disable?: [port names]} — the input devices' messages are also sent to these output ports as they are played",
            always,
            midi_thru
        ),
    ]
}

/// Longest port name kept in the session.
const MAX_PORT_CHARS: usize = 128;

fn track_output(e: &mut Engine, p: &Value) -> Result<Value> {
    const ID: &str = "midi.track_output";
    let tracks = tracks_required(e, ID, p)?;
    let port = match p.get("port") {
        Some(Value::Null) | None => None,
        Some(Value::String(name)) if !name.trim().is_empty() => {
            Some(name.chars().filter(|c| !c.is_control()).take(MAX_PORT_CHARS).collect::<String>())
        }
        Some(_) => return Err(bad(ID, "`port` must be a port name or null")),
    };
    for t in &tracks {
        match e.session().track(*t) {
            Some(tr) if tr.kind == TrackKind::Midi => {}
            Some(tr) => return Err(bad(ID, format!("`{}` is not a MIDI track", tr.name))),
            None => return Err(bad(ID, "unknown track")),
        }
    }
    let s = e.session_mut();
    for t in &tracks {
        if let Some(tr) = s.track_mut(*t) {
            tr.mixer.output = port.clone().map_or(Route::Main, Route::Hardware);
        }
    }
    Ok(json!({"port": port, "tracks": tracks.iter().map(|t| t.0).collect::<Vec<_>>()}))
}

fn thru_key(name: &str) -> String {
    let n: String = name.chars().filter(|c| !c.is_control() && *c != '=').take(MAX_PORT_CHARS).collect();
    format!("midi.thru.{}", n.trim())
}

fn midi_thru(e: &mut Engine, p: &Value) -> Result<Value> {
    let s = e.session_mut();
    for (key, on) in [("enable", true), ("disable", false)] {
        for n in p.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).take(64).collect::<Vec<_>>()).unwrap_or_default() {
            if !n.trim().is_empty() {
                s.edit.set_flag(&thru_key(n), on);
            }
        }
    }
    let enabled: Vec<&str> = s.edit.flags.iter().filter_map(|f| f.strip_prefix("midi.thru.")).collect();
    Ok(json!({"thru": enabled}))
}

/// The ports `e`'s session sends to: every hardware output of a MIDI track without an instrument,
/// and every Thru port (names as stored; the caller matches them against the system's ports).
pub fn wanted_output_names(e: &Engine) -> Vec<String> {
    let s = e.session();
    let mut names: Vec<String> = s
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Midi && t.instrument.is_none() && !t.inactive)
        .filter_map(|t| match &t.mixer.output {
            Route::Hardware(n) => Some(n.clone()),
            _ => None,
        })
        .collect();
    names.extend(s.edit.flags.iter().filter_map(|f| f.strip_prefix("midi.thru.")).map(str::to_string));
    names.sort();
    names.dedup();
    names
}

/// The Thru ports (by the name stored in the session).
pub fn thru_names(e: &Engine) -> Vec<String> {
    let mut v: Vec<String> = e.session().edit.flags.iter().filter_map(|f| f.strip_prefix("midi.thru.")).map(str::to_string).collect();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundcraft_model::ChannelFormat;

    #[test]
    fn midi_tracks_route_to_a_port_and_back_with_undo() {
        let mut e = Engine::default();
        let m = e.session_mut().add_track(TrackKind::Midi, ChannelFormat::Mono, None);
        let a = e.session_mut().add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        e.execute("midi.track_output", &json!({"tracks": [m.0], "port": "IAC Driver Bus 1"})).unwrap();
        assert_eq!(e.session().track(m).unwrap().mixer.output, Route::Hardware("IAC Driver Bus 1".into()));
        assert_eq!(wanted_output_names(&e), vec!["IAC Driver Bus 1".to_string()]);
        assert!(e.execute("midi.track_output", &json!({"tracks": [a.0], "port": "X"})).is_err(), "only MIDI tracks");
        assert!(e.execute("midi.track_output", &json!({"tracks": [m.0], "port": 5})).is_err());
        e.execute("midi.track_output", &json!({"tracks": [m.0], "port": null})).unwrap();
        assert_eq!(e.session().track(m).unwrap().mixer.output, Route::Main);
        assert!(wanted_output_names(&e).is_empty());
        assert!(e.undo());
        assert_eq!(e.session().track(m).unwrap().mixer.output, Route::Hardware("IAC Driver Bus 1".into()));
    }

    #[test]
    fn a_track_with_an_instrument_is_not_an_external_sender() {
        let mut e = Engine::default();
        let m = e.session_mut().add_track(TrackKind::Midi, ChannelFormat::Mono, None);
        e.execute("midi.track_output", &json!({"tracks": [m.0], "port": "Synth"})).unwrap();
        e.session_mut().track_mut(m).unwrap().instrument = Some(soundcraft_model::Insert::new("subtractive_synth"));
        assert!(wanted_output_names(&e).is_empty());
    }

    #[test]
    fn thru_ports_are_enabled_and_disabled_by_name() {
        let mut e = Engine::default();
        e.execute("setup.midi_thru", &json!({"enable": ["Synth A", "Synth B"]})).unwrap();
        assert_eq!(thru_names(&e), vec!["Synth A".to_string(), "Synth B".to_string()]);
        assert_eq!(wanted_output_names(&e), vec!["Synth A".to_string(), "Synth B".to_string()]);
        e.execute("setup.midi_thru", &json!({"disable": ["Synth A"]})).unwrap();
        assert_eq!(thru_names(&e), vec!["Synth B".to_string()]);
    }
}
