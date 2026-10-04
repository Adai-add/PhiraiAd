//! Recovered cutoff control with a portable stereo biquad implementation.
//! Unity's native DSP is not included in libil2cpp; this is not bit-identical DSP.
#[derive(Clone, Debug)]
pub struct LowPassTransition {
    pub cutoff: f32,
    enabled: bool,
    requested: bool,
    start: f32,
    target: f32,
    elapsed: f32,
    moving: bool,
}
impl Default for LowPassTransition {
    fn default() -> Self {
        Self {
            cutoff: 1500.,
            enabled: false,
            requested: false,
            start: 1500.,
            target: 1500.,
            elapsed: 0.,
            moving: false,
        }
    }
}
impl LowPassTransition {
    pub fn update(&mut self, on: bool, dt: f32) -> f32 {
        if on != self.requested {
            self.requested = on;
            self.start = self.cutoff;
            self.target = if on { 1500. } else { 22000. };
            self.elapsed = 0.;
            self.moving = true;
            if on {
                self.enabled = true
            }
        }
        if self.moving {
            if self.elapsed < 0.1 {
                self.elapsed += dt.max(0.);
                self.cutoff = self.start + (self.target - self.start) * (self.elapsed / 0.1).clamp(0., 1.);
            } else {
                self.cutoff = self.target;
                self.moving = false;
                if !self.requested {
                    self.enabled = false
                }
            }
        }
        if self.enabled {
            self.cutoff
        } else {
            0.
        }
    }
}
#[derive(Default)]
pub struct StereoLowPass {
    cutoff: f32,
    sr: u32,
    b: [f32; 3],
    a: [f32; 2],
    z: [[f32; 2]; 2],
}
impl StereoLowPass {
    pub fn reset(&mut self) {
        self.z = [[0.; 2]; 2];
    }
    pub fn configure(&mut self, cutoff: f32, sr: u32) {
        if self.cutoff == cutoff && self.sr == sr {
            return;
        }
        let was = self.cutoff;
        self.cutoff = cutoff;
        self.sr = sr;
        if cutoff <= 0. || was <= 0. {
            self.reset();
        }
        if cutoff <= 0. {
            return;
        }
        let w = std::f32::consts::TAU * cutoff.min(sr as f32 * 0.49) / sr.max(1) as f32;
        let (s, c) = w.sin_cos();
        let alpha = s / (2. * std::f32::consts::FRAC_1_SQRT_2);
        let a0 = 1. + alpha;
        self.b = [(1. - c) / 2. / a0, (1. - c) / a0, (1. - c) / 2. / a0];
        self.a = [-2. * c / a0, (1. - alpha) / a0];
    }
    pub fn process(&mut self, frame: sasa::Frame) -> sasa::Frame {
        if self.cutoff <= 0. {
            return frame;
        }
        let mut out = [0.; 2];
        for (i, x) in [frame.0, frame.1].into_iter().enumerate() {
            let y = self.b[0] * x + self.z[i][0];
            self.z[i][0] = self.b[1] * x - self.a[0] * y + self.z[i][1];
            self.z[i][1] = self.b[2] * x - self.a[1] * y;
            out[i] = y;
        }
        sasa::Frame(out[0], out[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cold_start_reentry_reversal() {
        let mut t = LowPassTransition::default();
        assert_eq!(t.update(false, 0.016), 0.);
        assert_eq!(t.update(true, 0.016), 1500.);
        t.update(false, 0.1);
        assert_eq!(t.cutoff, 22000.);
        assert_eq!(t.update(false, 0.016), 0.);
        let c = t.update(true, 0.05);
        assert_eq!(c, 11750.);
        let c = t.update(false, 0.05);
        assert_eq!(c, 16875.);
        assert_eq!(t.update(false, 0.1), 22000.);
        assert_eq!(t.update(false, 0.016), 0.);
    }
    #[test]
    fn stable_stereo_filter_and_bypass() {
        let mut f = StereoLowPass::default();
        f.configure(1500., 48000);
        let mut energy = 0.;
        for i in 0..48000 {
            let x = (i as f32 * std::f32::consts::TAU * 8000. / 48000.).sin();
            let y = f.process(sasa::Frame(x, x * 0.5));
            assert!(y.0.is_finite());
            assert!((y.1 - y.0 * 0.5).abs() < 0.00001);
            if i > 1000 {
                energy += y.0 * y.0;
            }
        }
        assert!(energy / 47000. < 0.01);
        f.configure(0., 48000);
        assert_eq!(f.process(sasa::Frame(0.4, 0.2)).0, 0.4);
    }
}
