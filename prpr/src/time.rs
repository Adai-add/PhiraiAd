//! Time manager for music time and real time synchronization.

use crate::config::Config;

pub struct TimeManager {
    pub adjust_time: bool,
    pub start_time: f64,
    pause_time: Option<f64>,
    pub speed: f64,
    pub force: f64,
    wait: f64,

    get_time_fn: Box<dyn Fn() -> f64>,
}

impl Default for TimeManager {
    fn default() -> Self {
        Self::new(1.0, false)
    }
}

impl TimeManager {
    pub fn from_config(config: &Config) -> Self {
        Self::new(1., config.adjust_time)
    }

    pub fn manual(get_time_fn: Box<dyn Fn() -> f64>) -> Self {
        let start_time = get_time_fn();
        Self {
            adjust_time: false,
            start_time,
            pause_time: None,
            speed: 1.0,
            wait: f64::NEG_INFINITY,
            force: 3e-3,

            get_time_fn,
        }
    }

    pub fn new(speed: f64, adjust_time: bool) -> Self {
        // we use performance.now() on web since audioContext.currentTime is not stable
        // and may cause serious latency problem
        #[cfg(target_arch = "wasm32")]
        let get_time_fn = {
            let perf = web_sys::window().unwrap().performance().unwrap();
            move || perf.now() / 1000.
        };
        #[cfg(not(target_arch = "wasm32"))]
        let get_time_fn = {
            let start = std::time::Instant::now();
            move || start.elapsed().as_secs_f64()
        };
        let t = get_time_fn();
        Self {
            adjust_time,
            start_time: t,
            pause_time: None,
            speed,
            wait: f64::NEG_INFINITY,
            force: 3e-3,

            get_time_fn: Box::new(get_time_fn),
        }
    }

    pub fn real_time(&self) -> f64 {
        (self.get_time_fn)()
    }

    pub fn reset(&mut self) {
        self.start_time = self.real_time();
        self.pause_time = None;
        self.wait = f64::NEG_INFINITY;
    }

    pub fn wait(&mut self) {
        self.wait = self.real_time() + 0.1;
    }

    pub fn dont_wait(&mut self) {
        self.wait = f64::NEG_INFINITY;
    }

    #[must_use]
    pub fn now(&self) -> f64 {
        (self.pause_time.unwrap_or_else(&self.get_time_fn) - self.start_time) * self.speed
    }

    pub fn update(&mut self, music_time: f64) {
        if self.adjust_time && self.real_time() > self.wait && self.pause_time.is_none() {
            self.start_time -= (music_time - self.now()) * self.force;
        }
    }

    #[must_use]
    pub fn paused(&self) -> bool {
        self.pause_time.is_some()
    }

    pub fn pause(&mut self) {
        if self.pause_time.is_none() {
            self.pause_time = Some(self.real_time());
        }
    }

    pub fn resume(&mut self) {
        if let Some(pause_time) = self.pause_time.take() {
            self.start_time += self.real_time() - pause_time;
            self.wait();
        }
    }

    pub fn seek_to(&mut self, pos: f64) {
        self.start_time = self.pause_time.unwrap_or_else(&self.get_time_fn) - pos / self.speed;
        self.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::TimeManager;
    use std::{cell::Cell, rc::Rc};

    fn clock() -> (TimeManager, Rc<Cell<f64>>) {
        let time = Rc::new(Cell::new(0.));
        let source = Rc::clone(&time);
        (TimeManager::manual(Box::new(move || source.get())), time)
    }

    #[test]
    fn initial_resume_keeps_running_clock() {
        let (mut tm, time) = clock();
        time.set(2.);
        tm.resume();
        assert!(!tm.paused());
        assert_eq!(tm.now(), 2.);
        assert_eq!(tm.start_time, 0.);
        assert_eq!(tm.wait, f64::NEG_INFINITY);
    }

    #[test]
    fn repeated_pause_preserves_original_frozen_time() {
        let (mut tm, time) = clock();
        time.set(2.);
        tm.pause();
        time.set(5.);
        tm.pause();
        assert!(tm.paused());
        assert_eq!(tm.now(), 2.);
        time.set(8.);
        tm.resume();
        assert!(!tm.paused());
        assert_eq!(tm.now(), 2.);
        time.set(9.);
        assert_eq!(tm.now(), 3.);
    }

    #[test]
    fn repeated_resume_does_not_shift_running_time() {
        let (mut tm, time) = clock();
        time.set(2.);
        tm.pause();
        time.set(5.);
        tm.resume();
        let wait = tm.wait;
        time.set(7.);
        tm.resume();
        assert_eq!(tm.start_time, 3.);
        assert_eq!(tm.now(), 4.);
        assert_eq!(tm.wait, wait);
    }

    #[test]
    fn paused_seek_and_speed_survive_resume() {
        let (mut tm, time) = clock();
        tm.speed = 2.;
        time.set(2.);
        tm.pause();
        tm.seek_to(10.);
        time.set(8.);
        tm.pause();
        assert_eq!(tm.now(), 10.);
        tm.resume();
        assert_eq!(tm.now(), 10.);
        time.set(9.);
        assert_eq!(tm.now(), 12.);
    }
}
