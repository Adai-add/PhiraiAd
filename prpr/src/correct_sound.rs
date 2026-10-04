//! Sample-clock hitsounds. Events are independent of judgement and render frames.
use sasa::{AudioClip, Frame};
use std::sync::Arc;

#[derive(Clone)]
pub struct ScheduledHit {
    pub time: f64,
    pub clip: AudioClip,
}
pub struct CorrectSoundTrack {
    events: Arc<Vec<ScheduledHit>>,
    offset: f64,
    next: usize,
    active: Vec<(usize, f64)>,
    volume: f32,
}
impl CorrectSoundTrack {
    pub fn new(events: Arc<Vec<ScheduledHit>>, offset: f64, volume: f32) -> Self {
        let mut track = Self {
            events,
            offset,
            next: 0,
            active: Vec::new(),
            volume,
        };
        track.seek(0.);
        track
    }
    pub fn seek(&mut self, song_time: f64) {
        self.active.clear();
        self.next = self.events.partition_point(|event| event.time + self.offset < song_time);
    }
    pub fn sample(&mut self, song_time: f64, rate: f64, sr: u32) -> Frame {
        while let Some(event) = self.events.get(self.next) {
            let start = event.time + self.offset;
            if start > song_time {
                break;
            }
            self.active.push((self.next, (song_time - start).max(0.) / rate));
            self.next += 1;
        }
        let mut output = Frame(0., 0.);
        self.active.retain_mut(|(index, elapsed)| {
            let Some(frame) = self.events[*index].clip.sample(*elapsed) else {
                return false;
            };
            output.0 += frame.0 * self.volume;
            output.1 += frame.1 * self.volume;
            *elapsed += 1. / sr as f64;
            true
        });
        output
    }
    pub fn finished(&self) -> bool {
        self.next == self.events.len() && self.active.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn track() -> CorrectSoundTrack {
        let clip = AudioClip::from_raw(vec![Frame(1., 0.5); 100], 1000);
        CorrectSoundTrack::new(
            Arc::new(vec![
                ScheduledHit {
                    time: 0.1,
                    clip: clip.clone(),
                },
                ScheduledHit { time: 0.1, clip },
            ]),
            0.05,
            0.2,
        )
    }
    #[test]
    fn offsets_chords_and_seeks_use_audio_samples() {
        let mut t = track();
        assert_eq!(t.sample(0.149, 1., 1000).0, 0.);
        assert!((t.sample(0.151, 1., 1000).0 - 0.4).abs() < 1e-6);
        t.seek(0.2);
        assert_eq!(t.sample(0.2, 1., 1000).0, 0.);
        t.seek(0.);
        assert!((t.sample(0.151, 1., 1000).0 - 0.4).abs() < 1e-6);
    }
    #[test]
    fn effect_duration_is_not_changed_by_playback_speed() {
        for rate in [0.05, 0.5, 1., 2., 10.] {
            let mut t = track();
            let start = 0.151;
            let frames: Vec<_> = (0..120).map(|i| t.sample(start + i as f64 * rate / 1000., rate, 1000).0).collect();
            let count = frames.iter().filter(|&&v| v != 0.).count();
            assert!(count >= 80 && count <= 101, "rate={rate}, frames={count}");
            assert!(t.finished());
        }
    }
}
