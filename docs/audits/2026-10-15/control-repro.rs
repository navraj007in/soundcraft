// Compile the production server with a minimal UI adapter. No SoundCraft window is opened.
extern crate self as egui;
extern crate self as soundcraft_ui_egui;

#[derive(Default, Clone)]
pub struct Context;
impl Context {
    pub fn request_repaint(&self) {}
}

pub struct ControlRequest {
    pub method: String,
    pub params: serde_json::Value,
    pub reply: std::sync::mpsc::Sender<serde_json::Value>,
}
impl ControlRequest {
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> (Self, std::sync::mpsc::Receiver<serde_json::Value>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { method: method.into(), params, reply: tx }, rx)
    }
}

#[path = "../../../apps/soundcraft/src/control_server.rs"]
mod control_server;
