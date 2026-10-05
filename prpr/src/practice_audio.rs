//! Streaming music for practice pitch preservation, noise filtering, and scheduled hitsounds.
//! Native sasa playback is kept when none of these processing paths is needed.
//! Streams bounded chunks from the already decoded clip; never expands the song
//! into a second full-length buffer. Positions and seeks always use song seconds.
use anyhow::{ensure, Context, Result};
use sasa::{AudioClip, AudioManager, Frame, MusicParams, Renderer};
use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc, Arc, OnceLock, Weak,
    },
};

pub static PREFERENCE_SAVER: OnceLock<fn(bool) -> Result<()>> = OnceLock::new();
pub fn save_preference(value: bool) -> Result<()> {
    if let Some(save) = PREFERENCE_SAVER.get() {
        save(value)?;
    }
    Ok(())
}
unsafe extern "C" {
    fn phira_tempo_new(sr: u32, tempo: f64) -> *mut c_void;
    fn phira_tempo_free(p: *mut c_void);
    fn phira_tempo_put(p: *mut c_void, data: *const f32, frames: u32) -> i32;
    fn phira_tempo_receive(p: *mut c_void, data: *mut f32, frames: u32) -> i32;
    fn phira_tempo_clear(p: *mut c_void) -> i32;
    fn phira_tempo_flush(p: *mut c_void) -> i32;
}
struct Tempo(NonNull<c_void>);
// The processor has one owner and is only accessed by the mixer thread after
// construction; SoundTouch's instance state is not shared with other threads.
unsafe impl Send for Tempo {}
impl Tempo {
    fn new(sr: u32, rate: f64) -> Result<Self> {
        Ok(Self(NonNull::new(unsafe { phira_tempo_new(sr, rate) }).context("无法初始化音高修复")?))
    }
}
impl Drop for Tempo {
    fn drop(&mut self) {
        unsafe {
            phira_tempo_free(self.0.as_ptr());
        }
    }
}
const BLOCK: usize = 1024;
struct PitchStream {
    tempo: Tempo,
    clip: AudioClip,
    sr: u32,
    rate: f64,
    input_frame: u64,
    flushed: bool,
    output: [f32; BLOCK * 2],
    cursor: usize,
    count: usize,
}
impl PitchStream {
    fn new(clip: AudioClip, rate: f64, sr: u32) -> Result<Self> {
        Ok(Self {
            tempo: Tempo::new(sr, rate)?,
            clip,
            sr,
            rate,
            input_frame: 0,
            flushed: false,
            output: [0.; BLOCK * 2],
            cursor: 0,
            count: 0,
        })
    }
    fn seek(&mut self, time: f64, sr: u32) -> Result<()> {
        if sr != self.sr {
            self.tempo = Tempo::new(sr, self.rate)?;
            self.sr = sr;
        } else {
            ensure!(unsafe { phira_tempo_clear(self.tempo.0.as_ptr()) } == 0, "音高修复重置失败");
        }
        self.input_frame = (time.max(0.) * sr as f64).round() as u64;
        self.cursor = 0;
        self.count = 0;
        self.flushed = false;
        Ok(())
    }
    fn next(&mut self) -> Result<Option<Frame>> {
        if self.cursor == self.count {
            loop {
                let n = unsafe { phira_tempo_receive(self.tempo.0.as_ptr(), self.output.as_mut_ptr(), BLOCK as u32) };
                ensure!(n >= 0, "音高修复输出失败");
                if n > 0 {
                    self.cursor = 0;
                    self.count = n as usize;
                    break;
                }
                if self.flushed {
                    return Ok(None);
                }
                let mut input = [0.; BLOCK * 2];
                let mut n = 0;
                while n < BLOCK {
                    let time = self.input_frame as f64 / self.sr as f64;
                    let Some(frame) = self.clip.sample(time) else {
                        break;
                    };
                    input[n * 2] = frame.0;
                    input[n * 2 + 1] = frame.1;
                    self.input_frame += 1;
                    n += 1;
                }
                if n == 0 {
                    ensure!(unsafe { phira_tempo_flush(self.tempo.0.as_ptr()) } == 0, "音高修复收尾失败");
                    self.flushed = true;
                } else {
                    ensure!(unsafe { phira_tempo_put(self.tempo.0.as_ptr(), input.as_ptr(), n as u32) } == 0, "音高修复输入失败");
                }
            }
        }
        let frame = Frame(self.output[self.cursor * 2], self.output[self.cursor * 2 + 1]);
        self.cursor += 1;
        Ok(Some(frame))
    }
}
#[derive(Default)]
struct Shared {
    position: AtomicU64,
    paused: AtomicBool,
    cutoff: AtomicU32,
}
enum Command {
    Play,
    Pause,
    Seek(f64),
}
struct Corrected {
    stream: PitchStream,
    pitch_enabled: bool,
    filter: crate::noise_area::audio::StereoLowPass,
    correct_sound: Option<crate::correct_sound::CorrectSoundTrack>,
    shared: Weak<Shared>,
    rx: mpsc::Receiver<Command>,
    paused: bool,
    position: f64,
    amplifier: f32,
}
impl Corrected {
    fn prepare(&mut self, sr: u32) -> bool {
        let mut seek = None;
        while let Ok(cmd) = self.rx.try_recv() {
            match cmd {
                Command::Play => self.paused = false,
                Command::Pause => self.paused = true,
                Command::Seek(time) => {
                    self.position = time.max(0.);
                    seek = Some(self.position);
                }
            }
        }
        if sr != self.stream.sr {
            seek = Some(self.position);
        }
        if let Some(time) = seek {
            self.filter.reset();
            if let Some(track) = &mut self.correct_sound {
                track.seek(time);
            }
            if self.stream.seek(time, sr).is_err() {
                self.paused = true;
            }
        }
        if let Some(s) = self.shared.upgrade() {
            self.filter.configure(f32::from_bits(s.cutoff.load(Ordering::Relaxed)), sr);
        }
        self.publish();
        !self.paused
    }
    fn publish(&self) {
        if let Some(s) = self.shared.upgrade() {
            s.position
                .store(self.position.min(self.stream.clip.length()).to_bits(), Ordering::Relaxed);
            s.paused.store(self.paused, Ordering::Relaxed);
        }
    }
    fn frame(&mut self) -> Option<Frame> {
        let music_finished = self.position >= self.stream.clip.length();
        if music_finished && self.correct_sound.as_ref().is_none_or(|track| track.finished()) {
            self.paused = true;
            return None;
        }
        let effects = self
            .correct_sound
            .as_mut()
            .map(|track| track.sample(self.position, self.stream.rate, self.stream.sr))
            .unwrap_or(Frame(0., 0.));
        let frame = if music_finished {
            Ok(Some(Frame(0., 0.)))
        } else if self.pitch_enabled {
            self.stream.next()
        } else {
            Ok(self.stream.clip.sample(self.position))
        };
        match frame {
            Ok(Some(frame)) => {
                self.position += self.stream.rate / self.stream.sr as f64;
                let music = self.filter.process(frame) * self.amplifier;
                Some(Frame(music.0 + effects.0, music.1 + effects.1))
            }
            _ => {
                self.paused = true;
                None
            }
        }
    }
}
impl Renderer for Corrected {
    fn alive(&self) -> bool {
        self.shared.strong_count() != 0
    }
    fn render_mono(&mut self, sr: u32, data: &mut [f32]) {
        if self.prepare(sr) {
            for out in data {
                let Some(f) = self.frame() else {
                    break;
                };
                *out += f.avg();
            }
            self.publish();
        }
    }
    fn render_stereo(&mut self, sr: u32, data: &mut [f32]) {
        if self.prepare(sr) {
            for out in data.chunks_exact_mut(2) {
                let Some(f) = self.frame() else {
                    break;
                };
                out[0] += f.0;
                out[1] += f.1;
            }
            self.publish();
        }
    }
}
enum Backend {
    Native(sasa::Music),
    Corrected { shared: Arc<Shared>, tx: mpsc::SyncSender<Command> },
}
pub struct Music {
    backend: Backend,
    preserve_pitch: bool,
    noise_filter: crate::noise_area::audio::LowPassTransition,
}
impl Music {
    pub fn new(
        audio: &mut AudioManager,
        clip: AudioClip,
        params: MusicParams,
        preserve_pitch: bool,
        noise_filter: bool,
        correct_sound: Option<crate::correct_sound::CorrectSoundTrack>,
    ) -> Result<Self> {
        Self::new_inner(audio, clip, params, preserve_pitch, noise_filter, correct_sound, false)
    }
    /// Replays must never block the UI on sasa's native synchronous command queue.
    pub fn new_replay(
        audio: &mut AudioManager,
        clip: AudioClip,
        params: MusicParams,
        preserve_pitch: bool,
        noise_filter: bool,
        correct_sound: Option<crate::correct_sound::CorrectSoundTrack>,
    ) -> Result<Self> {
        Self::new_inner(audio, clip, params, preserve_pitch, noise_filter, correct_sound, true)
    }
    #[allow(clippy::too_many_arguments)]
    fn new_inner(
        audio: &mut AudioManager,
        clip: AudioClip,
        params: MusicParams,
        preserve_pitch: bool,
        noise_filter: bool,
        correct_sound: Option<crate::correct_sound::CorrectSoundTrack>,
        replay: bool,
    ) -> Result<Self> {
        let backend = if !replay && correct_sound.is_none() && !noise_filter && (!preserve_pitch || (params.playback_rate - 1.).abs() < 1e-6) {
            Backend::Native(audio.create_music(clip, params)?)
        } else {
            let stream = PitchStream::new(clip.clone(), params.playback_rate, clip.sample_rate())?;
            let shared = Arc::new(Shared {
                paused: AtomicBool::new(true),
                ..Default::default()
            });
            let (tx, rx) = mpsc::sync_channel(params.command_buffer_size);
            audio.add_renderer(Corrected {
                stream,
                pitch_enabled: preserve_pitch && (params.playback_rate - 1.).abs() >= 1e-6,
                filter: Default::default(),
                correct_sound,
                shared: Arc::downgrade(&shared),
                rx,
                paused: true,
                position: 0.,
                amplifier: params.amplifier,
            })?;
            Backend::Corrected { shared, tx }
        };
        Ok(Self {
            backend,
            preserve_pitch,
            noise_filter: Default::default(),
        })
    }
    pub fn set_noise_touch(&mut self, touching: bool, dt: f32) {
        let cutoff = self.noise_filter.update(touching, dt);
        if let Backend::Corrected { shared, .. } = &self.backend {
            shared.cutoff.store(cutoff.to_bits(), Ordering::Relaxed);
        }
    }
    pub fn preserves_pitch(&self) -> bool {
        self.preserve_pitch
    }
    pub fn position(&self) -> f64 {
        match &self.backend {
            Backend::Native(m) => m.position(),
            Backend::Corrected { shared, .. } => f64::from_bits(shared.position.load(Ordering::Relaxed)),
        }
    }
    pub fn paused(&mut self) -> bool {
        match &mut self.backend {
            Backend::Native(m) => m.paused(),
            Backend::Corrected { shared, .. } => shared.paused.load(Ordering::Relaxed),
        }
    }
    pub fn play(&mut self) -> Result<()> {
        match &mut self.backend {
            Backend::Native(m) => m.play(),
            Backend::Corrected { tx, .. } => Ok(tx.try_send(Command::Play)?),
        }
    }
    pub fn pause(&mut self) -> Result<()> {
        match &mut self.backend {
            Backend::Native(m) => m.pause(),
            Backend::Corrected { tx, .. } => Ok(tx.try_send(Command::Pause)?),
        }
    }
    pub fn seek_to(&mut self, time: f64) -> Result<()> {
        self.noise_filter = Default::default();
        if let Backend::Corrected { shared, .. } = &self.backend {
            shared.cutoff.store(0, Ordering::Relaxed);
        }
        match &mut self.backend {
            Backend::Native(m) => m.seek_to(time),
            Backend::Corrected { tx, .. } => Ok(tx.try_send(Command::Seek(time))?),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_audio_command_queue_returns_instead_of_blocking_ui() {
        let (tx, _rx) = mpsc::sync_channel(1);
        tx.try_send(Command::Pause).unwrap();
        let mut music = Music {
            backend: Backend::Corrected {
                shared: Arc::new(Shared::default()),
                tx,
            },
            preserve_pitch: false,
            noise_filter: Default::default(),
        };
        assert!(music.play().is_err());
        assert!(music.pause().is_err());
        assert!(music.seek_to(0.).is_err());
    }
    #[test]
    fn replay_countdown_and_pause_do_not_flood_unacknowledged_audio_queue() {
        let (tx, rx) = mpsc::sync_channel(2);
        let mut music = Music {
            backend: Backend::Corrected {
                shared: Arc::new(Shared::default()),
                tx,
            },
            preserve_pitch: false,
            noise_filter: Default::default(),
        };
        let mut requested = false;
        // No mixer acknowledgements: simulate 3 seconds count-in, playback,
        // pause, and hundreds of idle frames after the end of a replay.
        for audible in std::iter::repeat_n(false, 360)
            .chain(std::iter::repeat_n(true, 360))
            .chain(std::iter::repeat_n(false, 360))
        {
            if let Some(play) = crate::replay::audio_transition(requested, audible) {
                if play {
                    music.play().unwrap();
                } else {
                    music.pause().unwrap();
                }
                requested = play;
            }
        }
        assert!(matches!(rx.try_recv().unwrap(), Command::Play));
        assert!(matches!(rx.try_recv().unwrap(), Command::Pause));
        assert!(rx.try_recv().is_err());
    }
    fn tone(seconds: f64, sr: u32, frequency: f64) -> AudioClip {
        AudioClip::from_raw(
            (0..(seconds * sr as f64) as usize)
                .map(|i| {
                    let v = (i as f64 * frequency * std::f64::consts::TAU / sr as f64).sin() as f32 * 0.4;
                    Frame(v, v * 0.5)
                })
                .collect(),
            sr,
        )
    }
    fn corrected(clip: AudioClip, rate: f64) -> (Corrected, Arc<Shared>, mpsc::SyncSender<Command>) {
        let sr = clip.sample_rate();
        let shared = Arc::new(Shared::default());
        let (tx, rx) = mpsc::sync_channel(16);
        (
            Corrected {
                stream: PitchStream::new(clip, rate, sr).unwrap(),
                pitch_enabled: true,
                filter: Default::default(),
                correct_sound: None,
                shared: Arc::downgrade(&shared),
                rx,
                paused: true,
                position: 0.,
                amplifier: 1.,
            },
            shared,
            tx,
        )
    }
    #[test]
    fn scheduled_effects_share_the_music_clock_and_survive_pause_and_seek() {
        let sr = 48000;
        let (mut renderer, shared, tx) = corrected(tone(0.5, sr, 440.), 1.);
        renderer.pitch_enabled = false;
        renderer.amplifier = 0.; // Muted music must not mute scheduled hitsounds.
        let effect = AudioClip::from_raw(vec![Frame(1., 0.5); 480], sr);
        renderer.correct_sound = Some(crate::correct_sound::CorrectSoundTrack::new(
            Arc::new(vec![crate::correct_sound::ScheduledHit { time: 0.01, clip: effect }]),
            0.,
            0.3,
        ));
        tx.try_send(Command::Play).unwrap();
        let mut first = vec![0.; 4800];
        renderer.render_stereo(sr, &mut first);
        let onset = first.chunks_exact(2).position(|frame| frame[0] > 0.1).unwrap();
        assert!((480..=481).contains(&onset));
        assert!((first[onset * 2] - 0.3).abs() < 1e-6);
        assert!((first[onset * 2 + 1] - 0.15).abs() < 1e-6);
        tx.try_send(Command::Pause).unwrap();
        let before = renderer.position;
        let mut paused = vec![0.; 4800];
        renderer.render_stereo(sr, &mut paused);
        assert!(paused.iter().all(|value| *value == 0.));
        assert_eq!(renderer.position, before);
        tx.try_send(Command::Seek(0.)).unwrap();
        tx.try_send(Command::Play).unwrap();
        let mut replay = vec![0.; 4800];
        renderer.render_stereo(sr, &mut replay);
        assert_eq!(first, replay);
        assert!((f64::from_bits(shared.position.load(Ordering::Relaxed)) - 0.05).abs() < 1e-6);
    }
    fn frequency(data: &[f32], sr: u32) -> f64 {
        let crossings = data.windows(2).filter(|v| v[0] <= 0. && v[1] > 0.).count();
        crossings as f64 * sr as f64 / data.len() as f64
    }
    #[test]
    fn pitch_and_duration_across_practice_rates() {
        let sr = 8000;
        for rate in [0.05, 0.5, 0.75, 1.25, 2., 10.] {
            let (mut r, _shared, tx) = corrected(tone(2., sr, 440.), rate);
            tx.try_send(Command::Play).unwrap();
            assert!(r.prepare(sr));
            let mut output = Vec::new();
            while let Some(frame) = r.frame() {
                assert!((frame.1 - frame.0 * 0.5).abs() < 1e-5, "stereo phase at {rate}");
                output.push(frame.0);
                assert!(output.len() < 800_000);
            }
            let expected = (2. * sr as f64 / rate).round() as usize;
            assert!(output.len().abs_diff(expected) <= 2, "duration at {rate}: {} vs {expected}", output.len());
            let center = &output[output.len() / 4..output.len() * 3 / 4];
            let hz = frequency(center, sr);
            assert!((hz - 440.).abs() < 12., "pitch at {rate}: {hz}");
            assert!(center.iter().map(|v| v * v).sum::<f32>() / center.len() as f32 > 0.02);
        }
    }
    #[test]
    fn pause_seek_resume_and_sample_rate_change_keep_song_clock() {
        let (mut r, shared, tx) = corrected(tone(2., 8000, 440.), 0.5);
        let mut out = vec![0.; 1600];
        r.render_stereo(8000, &mut out);
        assert!(out.iter().all(|x| *x == 0.));
        tx.try_send(Command::Play).unwrap();
        r.render_stereo(8000, &mut out);
        assert!((r.position - 0.05).abs() < 1e-7);
        tx.try_send(Command::Pause).unwrap();
        r.render_stereo(8000, &mut out);
        assert!((r.position - 0.05).abs() < 1e-7);
        tx.send(Command::Seek(0.6)).unwrap();
        tx.send(Command::Seek(1.)).unwrap(); // latest drag position wins
        r.render_stereo(8000, &mut out);
        assert_eq!(r.position, 1.);
        assert_eq!(f64::from_bits(shared.position.load(Ordering::Relaxed)), 1.);
        tx.try_send(Command::Play).unwrap();
        r.render_stereo(16000, &mut out);
        assert!((r.position - 1.025).abs() < 1e-7);
        tx.try_send(Command::Pause).unwrap();
        tx.send(Command::Seek(0.)).unwrap();
        r.render_stereo(16000, &mut out);
        assert_eq!(r.position, 0.);
        tx.try_send(Command::Play).unwrap();
        r.render_stereo(16000, &mut out);
        assert!((r.position - 0.025).abs() < 1e-7);
    }
    #[test]
    fn seek_discards_old_pitch_buffer_and_reaches_exact_end() {
        let sr = 8000;
        let mut frames = tone(1., sr, 220.).frames().to_vec();
        frames.extend_from_slice(tone(1., sr, 660.).frames());
        let (mut r, _shared, tx) = corrected(AudioClip::from_raw(frames, sr), 0.5);
        tx.try_send(Command::Play).unwrap();
        r.prepare(sr);
        for _ in 0..100 {
            r.frame();
        }
        tx.send(Command::Seek(1.2)).unwrap();
        r.prepare(sr);
        let mut output = Vec::new();
        while let Some(f) = r.frame() {
            output.push(f.0);
        }
        assert!((frequency(&output[800..output.len() - 800], sr) - 660.).abs() < 3.);
        assert!((r.position - 2.).abs() < 1e-7);
        assert!(r.paused);
    }
}
