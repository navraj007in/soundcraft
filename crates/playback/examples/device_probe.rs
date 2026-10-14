//! Audio device probe (plan task P0-T05): lists the devices and what they support, then plays
//! SILENCE on the default output at several requested buffer sizes and reports the callback sizes
//! the device actually delivers. Compare with `BLOCK = 512` in `crates/playback/src/lib.rs`.
//!
//! It never builds an input stream, so it cannot start recording or ask for microphone access;
//! input devices are only listed with the configurations they report.
//!
//!   cargo run -p soundcraft-playback --example device_probe
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SupportedBufferSize};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[allow(deprecated)]
fn name(d: &cpal::Device) -> String {
    d.name().unwrap_or_else(|_| "?".into())
}

fn buffer(b: &SupportedBufferSize) -> String {
    match b {
        SupportedBufferSize::Range { min, max } => format!("{min}..{max} frames"),
        SupportedBufferSize::Unknown => "unknown".into(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    println!("host: {:?}", host.id());
    let default_out = host.default_output_device().map(|d| name(&d));
    let default_in = host.default_input_device().map(|d| name(&d));
    println!("default output: {default_out:?}\ndefault input:  {default_in:?}\n");

    for (kind, devices) in [("OUTPUT", host.output_devices()?.collect::<Vec<_>>()), ("INPUT", host.input_devices()?.collect::<Vec<_>>())] {
        for d in devices {
            println!("{kind} device `{}`", name(&d));
            let configs: Vec<_> = if kind == "OUTPUT" { d.supported_output_configs()?.collect() } else { d.supported_input_configs()?.collect() };
            for c in configs {
                println!(
                    "   {:?} {} ch, {}..{} Hz, buffer {}",
                    c.sample_format(),
                    c.channels(),
                    c.min_sample_rate().0,
                    c.max_sample_rate().0,
                    buffer(c.buffer_size())
                );
            }
        }
    }

    let Some(device) = host.default_output_device() else { return Ok(()) };
    let default = device.default_output_config()?;
    println!(
        "\nsilent callback-size probe on `{}` (default config {:?} {} ch {} Hz)",
        name(&device),
        default.sample_format(),
        default.channels(),
        default.sample_rate().0
    );
    println!("   requested | callbacks |   min median   max | all equal / note");
    if default.sample_format() != cpal::SampleFormat::F32 {
        println!("default format is not f32; the probe only supports f32");
        return Ok(());
    }
    for request in [None, Some(32u32), Some(64), Some(128), Some(256), Some(512), Some(1024)] {
        let mut config = default.config();
        config.buffer_size = request.map_or(BufferSize::Default, BufferSize::Fixed);
        let sizes: Arc<Mutex<Vec<usize>>> = Arc::default();
        let (log, ch) = (Arc::clone(&sizes), usize::from(config.channels).max(1));
        let stream = device.build_output_stream(
            &config,
            move |data: &mut [f32], _| {
                data.fill(0.0);
                if let Ok(mut v) = log.lock() {
                    v.push(data.len() / ch);
                }
            },
            |e| eprintln!("stream error: {e}"),
            None,
        );
        let label = request.map_or("default".to_string(), |n| n.to_string());
        match stream {
            Err(e) => println!("{label:>12} | refused: {e}"),
            Ok(s) => {
                s.play()?;
                std::thread::sleep(Duration::from_millis(1500));
                drop(s);
                let mut v = sizes.lock().map(|v| v.clone()).unwrap_or_default();
                v.sort_unstable();
                if v.is_empty() {
                    println!("{label:>12} | no callbacks");
                    continue;
                }
                let (min, med, max) = (v[0], v[v.len() / 2], v[v.len() - 1]);
                println!(
                    "{label:>12} | {:>9} | {min:>5} {med:>6} {max:>5} | {}",
                    v.len(),
                    if min == max { "yes".to_string() } else { "no: sizes vary".to_string() }
                );
            }
        }
    }
    Ok(())
}
