//! Session-level settings the UI used to write straight into the document: the session comments and
//! which automation parameters are enabled for writing. Going through commands gives them the
//! journal, undo and error boundary every other edit has.

use super::*;
use crate::cmd;
use serde_json::json;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("session.set_comments", "Edit Session Comments", [], None, "{comments} → {length}. Capped at 64 KiB.", always, set_comments),
        cmd!(
            noundo "automation.enable_parameter",
            "Enable Automation Parameter",
            [],
            None,
            "{parameter: volume|pan|mute|send level|send pan|send mute|plugin, enabled?: bool (default: toggle)}",
            always,
            enable_parameter
        ),
    ]
}

/// Longest comment kept (bytes): a hostile or runaway paste must not bloat every undo snapshot.
const MAX_COMMENTS: usize = 64 * 1024;

fn set_comments(e: &mut Engine, p: &Value) -> Result<Value> {
    let c = str_param(p, "comments").ok_or_else(|| bad("session.set_comments", "`comments` required"))?;
    if c.len() > MAX_COMMENTS {
        return Err(bad("session.set_comments", format!("comments are limited to {MAX_COMMENTS} bytes")));
    }
    let c = c.to_string();
    let len = c.len();
    e.session_mut().comments = c;
    Ok(json!({"length": len}))
}

const PARAMETERS: [&str; 7] = ["volume", "pan", "mute", "send level", "send pan", "send mute", "plugin"];

fn enable_parameter(e: &mut Engine, p: &Value) -> Result<Value> {
    let name = str_param(p, "parameter").ok_or_else(|| bad("automation.enable_parameter", "`parameter` required"))?;
    if !PARAMETERS.contains(&name) {
        return Err(bad("automation.enable_parameter", format!("unknown parameter `{name}`")));
    }
    let off = format!("automation.enable.{}.off", name.replace(' ', "_"));
    let s = e.session_mut();
    let enabled = p.get("enabled").and_then(Value::as_bool).unwrap_or_else(|| s.edit.flag(&off));
    s.edit.set_flag(&off, !enabled);
    Ok(json!({"parameter": name, "enabled": enabled}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_edit_through_a_command_with_undo_and_a_cap() {
        let mut e = Engine::default();
        e.execute("session.set_comments", &json!({"comments": "Take 3 is the keeper"})).unwrap();
        assert_eq!(e.session().comments, "Take 3 is the keeper");
        assert!(e.undo());
        assert_eq!(e.session().comments, "");
        assert!(e.execute("session.set_comments", &json!({"comments": "x".repeat(MAX_COMMENTS + 1)})).is_err());
        assert!(e.execute("session.set_comments", &json!({})).is_err());
    }

    #[test]
    fn typing_is_one_undo_step_when_merged() {
        let mut e = Engine::default();
        for text in ["h", "he", "hel", "hell", "hello"] {
            e.execute_merged("session.set_comments", &json!({"comments": text}), "comments").unwrap();
        }
        e.end_merge();
        assert_eq!(e.undo_history().len(), 1);
        assert!(e.undo());
        assert_eq!(e.session().comments, "");
    }

    #[test]
    fn automation_parameters_enable_and_disable() {
        let mut e = Engine::default();
        let on = |e: &Engine| !e.session().edit.flag("automation.enable.send_level.off");
        assert!(on(&e));
        e.execute("automation.enable_parameter", &json!({"parameter": "send level", "enabled": false})).unwrap();
        assert!(!on(&e));
        e.execute("automation.enable_parameter", &json!({"parameter": "send level"})).unwrap();
        assert!(on(&e), "no `enabled` toggles");
        assert!(e.execute("automation.enable_parameter", &json!({"parameter": "bogus"})).is_err());
    }
}
