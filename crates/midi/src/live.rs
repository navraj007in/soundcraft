//! Turning a performance's raw channel messages into a [`Sequence`].

use crate::{CtrlEvent, CtrlKind, Note, Sequence};
use std::collections::HashMap;

/// Most notes and controller events one take keeps (a hostile or runaway device must not grow a
/// session without bound).
const MAX_EVENTS: usize = 2_000_000;

/// A controller or note message decoded from raw bytes.
enum Decoded {
    NoteOn { channel: u8, pitch: u8, velocity: u8 },
    NoteOff { channel: u8, pitch: u8, velocity: u8 },
    Ctrl { channel: u8, kind: CtrlKind, value: i32 },
}

fn decode(msg: [u8; 3]) -> Option<Decoded> {
    let channel = msg[0] & 0x0F;
    let (d1, d2) = (msg[1] & 0x7F, msg[2] & 0x7F);
    Some(match msg[0] & 0xF0 {
        0x90 if d2 > 0 => Decoded::NoteOn { channel, pitch: d1, velocity: d2 },
        // Note-on with velocity 0 is a note-off.
        0x90 => Decoded::NoteOff { channel, pitch: d1, velocity: 64 },
        0x80 => Decoded::NoteOff { channel, pitch: d1, velocity: d2 },
        0xA0 => Decoded::Ctrl { channel, kind: CtrlKind::PolyPressure(d1), value: i32::from(d2) },
        0xB0 => Decoded::Ctrl { channel, kind: CtrlKind::Cc(d1), value: i32::from(d2) },
        0xC0 => Decoded::Ctrl { channel, kind: CtrlKind::Program, value: i32::from(d1) },
        0xD0 => Decoded::Ctrl { channel, kind: CtrlKind::ChannelPressure, value: i32::from(d1) },
        // 14-bit pitch bend, centred on 0.
        0xE0 => Decoded::Ctrl { channel, kind: CtrlKind::PitchBend, value: (i32::from(d2) << 7 | i32::from(d1)) - 8192 },
        _ => return None,
    })
}

impl Sequence {
    /// The sequence a performance made of `events` (a tick and a channel message each, in the order
    /// they arrived) records. A note runs from its note-on to the next note-off of the same pitch
    /// and channel; one still held at `end_tick` is ended there. A repeated note-on of a sounding
    /// pitch ends the earlier note first. A note-off with no note-on is ignored. Controllers keep
    /// their order. Ticks before 0 are clamped to 0.
    pub fn from_performance(events: &[(i64, [u8; 3])], end_tick: i64) -> Sequence {
        let mut seq = Sequence::default();
        let mut held: HashMap<(u8, u8), (i64, u8)> = HashMap::new();
        let close = |seq: &mut Sequence, channel: u8, pitch: u8, start: i64, velocity: u8, end: i64, release: u8| {
            if seq.notes.len() < MAX_EVENTS {
                seq.notes.push(Note { pitch, velocity, release_velocity: release, channel, start, length: (end - start).max(1) });
            }
        };
        for &(tick, msg) in events {
            let tick = tick.max(0);
            match decode(msg) {
                Some(Decoded::NoteOn { channel, pitch, velocity }) => {
                    if let Some((start, vel)) = held.insert((channel, pitch), (tick, velocity)) {
                        close(&mut seq, channel, pitch, start, vel, tick, 64);
                    }
                }
                Some(Decoded::NoteOff { channel, pitch, velocity }) => {
                    if let Some((start, vel)) = held.remove(&(channel, pitch)) {
                        close(&mut seq, channel, pitch, start, vel, tick, velocity);
                    }
                }
                Some(Decoded::Ctrl { channel, kind, value }) if seq.ctrls.len() < MAX_EVENTS => {
                    seq.ctrls.push(CtrlEvent { tick, channel, kind, value });
                }
                _ => {}
            }
        }
        let mut still: Vec<_> = held.into_iter().collect();
        still.sort_unstable();
        for ((channel, pitch), (start, vel)) in still {
            close(&mut seq, channel, pitch, start, vel, end_tick.max(start + 1), 64);
        }
        seq.sort();
        seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_pair_up_by_pitch_and_channel() {
        let seq = Sequence::from_performance(
            &[(0, [0x90, 60, 100]), (100, [0x90, 64, 90]), (480, [0x80, 60, 40]), (600, [0x90, 64, 0]), (700, [0x91, 60, 70]), (900, [0x81, 60, 0])],
            1000,
        );
        let n = &seq.notes;
        assert_eq!(n.len(), 3);
        assert_eq!((n[0].pitch, n[0].channel, n[0].start, n[0].length, n[0].velocity, n[0].release_velocity), (60, 0, 0, 480, 100, 40));
        assert_eq!((n[1].pitch, n[1].start, n[1].length), (64, 100, 500), "velocity-0 note-on ends the note");
        assert_eq!((n[2].channel, n[2].start, n[2].length), (1, 700, 200), "the same pitch on another channel is another note");
    }

    #[test]
    fn a_held_note_is_ended_at_the_end_of_the_take_and_a_stray_off_is_ignored() {
        let seq = Sequence::from_performance(&[(10, [0x80, 50, 0]), (200, [0x90, 72, 100])], 1000);
        assert_eq!(seq.notes.len(), 1);
        assert_eq!((seq.notes[0].start, seq.notes[0].length), (200, 800));
    }

    #[test]
    fn a_repeated_note_on_ends_the_earlier_note() {
        let seq = Sequence::from_performance(&[(0, [0x90, 60, 100]), (300, [0x90, 60, 80]), (500, [0x80, 60, 0])], 600);
        assert_eq!(seq.notes.iter().map(|n| (n.start, n.length, n.velocity)).collect::<Vec<_>>(), vec![(0, 300, 100), (300, 200, 80)]);
    }

    #[test]
    fn controllers_bend_and_program_decode() {
        let seq = Sequence::from_performance(
            &[(0, [0xB0, 64, 127]), (10, [0xE0, 0, 64]), (20, [0xE0, 127, 127]), (30, [0xC2, 9, 0]), (40, [0xD0, 33, 0]), (50, [0xA0, 60, 12])],
            100,
        );
        let k: Vec<_> = seq.ctrls.iter().map(|c| (c.tick, c.channel, c.kind, c.value)).collect();
        assert_eq!(
            k,
            vec![
                (0, 0, CtrlKind::Cc(64), 127),
                (10, 0, CtrlKind::PitchBend, 0),
                (20, 0, CtrlKind::PitchBend, 8191),
                (30, 2, CtrlKind::Program, 9),
                (40, 0, CtrlKind::ChannelPressure, 33),
                (50, 0, CtrlKind::PolyPressure(60), 12),
            ]
        );
    }

    #[test]
    fn hostile_input_is_bounded_and_never_panics() {
        let mut events = Vec::new();
        for i in 0..50_000i64 {
            events.push((i64::MIN + i, [0x90 | (i % 16) as u8, (i % 256) as u8, 255]));
            events.push((i64::MAX - i, [0xFF, 0xFF, 0xFF]));
            events.push((i, [(i % 256) as u8, 200, 200]));
        }
        let seq = Sequence::from_performance(&events, i64::MIN);
        assert!(seq.notes.iter().all(|n| n.length >= 1 && n.start >= 0));
    }
}
