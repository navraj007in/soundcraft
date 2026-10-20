//! State capture through the real `Player` and a real hosted CLAP plugin (the fixture gain built
//! from `crates/clap-host/tests/fixture`): the state is read on the calling thread through the
//! handle the instance leaves behind, without involving the audio thread.

use soundcraft_model::{ChannelFormat, Insert, Session, TrackKind};
use soundcraft_playback::Player;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

fn install_fixture() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../clap-host/tests/fixture/Cargo.toml");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("clap-fixture-target");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let st = Command::new(cargo)
        .args(["build", "--quiet", "--manifest-path"])
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target)
        .env_remove("RUSTFLAGS")
        .status()
        .expect("cargo runs");
    assert!(st.success());
    let lib = if cfg!(target_os = "macos") {
        "libclap_fixture.dylib"
    } else if cfg!(windows) {
        "clap_fixture.dll"
    } else {
        "libclap_fixture.so"
    };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("playback-clap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::copy(target.join("debug").join(lib), dir.join("Fixture.clap")).expect("copy fixture");
    assert!(soundcraft_clap_host::add_search_dir(&dir) >= 1, "the fixture plug-ins are found");
}

#[test]
fn a_real_plugin_is_captured_without_involving_the_audio_thread() {
    install_fixture();
    let mut s = Session::default();
    let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
    s.track_mut(t).expect("track").mixer.inserts[0] = Some(Insert::new("clap:org.soundcraft.test.gain"));
    let player = Player::new(Arc::new(s.clone()));
    player.update_session(Arc::new(s));
    // Capture twice: the plugin is still live and gives the same state both times.
    let mut first = Vec::new();
    for _ in 0..50 {
        first = player.capture_states();
        if !first.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(first.len(), 1, "the fixture has state");
    assert_eq!((first[0].0, first[0].1), (t, 0));
    let second = player.capture_states();
    assert_eq!(second, first);
}
