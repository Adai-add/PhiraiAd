//! Frame-based Phigros matching and scoring, independent of rendering/input APIs.
//! Note.isJudged and the Drag/Flick Control flags deliberately remain separate.

pub(crate) const CANDIDATE_EPSILON: f64 = 0.010;
const DRAG_WINDOW: f64 = 0.100;
const RESOLVE_EARLY: f64 = 0.005;
const HOLD_TAIL: f64 = 0.220;
const HOLD_LATE_MISS: f64 = 0.250;

/// prpr uses 2 / viewport_width normalized units per physical pixel. Phigros
/// uses screen_height / 10 pixels per world unit, including letterboxed play.
pub(crate) fn world_unit(screen_height: f64, viewport_width: f64) -> f64 {
    screen_height / (5. * viewport_width)
}

pub(crate) fn flick_speeds(previous: [f32; 2], current: [f32; 2], frame_delta: f32) -> (f32, f32) {
    let previous_length = previous[0].hypot(previous[1]);
    let projection = if previous_length > 0.1 {
        (previous[0] * current[0] + previous[1] * current[1]) / previous_length
    } else {
        0.
    };
    let divisor = 60. * frame_delta.max(1e-6);
    (projection / divisor, current[0].hypot(current[1]) / divisor)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Tap,
    Hold,
    Drag,
    Flick,
}

#[derive(Clone, Copy)]
pub(crate) struct Windows {
    pub perfect: f64,
    pub good: f64,
    pub bad: f64,
    pub flick: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Perfect,
    Good,
    Bad,
    Miss,
}

struct State {
    clicked: bool,
    matched: bool,
    head: Option<bool>,
    head_time: f64,
    safe_frames: i8,
    outcome: Option<Outcome>,
    skipped: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            clicked: false,
            matched: false,
            head: None,
            head_time: 0.,
            safe_frames: 2,
            outcome: None,
            skipped: false,
        }
    }
}

pub(crate) struct Note {
    pub line_id: usize,
    pub note_id: u32,
    pub time: f64,
    pub end_time: f64,
    pub kind: Kind,
    /// Animated note x in Phigros world units, refreshed by the adapter.
    pub x: f64,
    state: State,
}

impl Note {
    pub fn new(line_id: usize, note_id: u32, time: f64, end_time: f64, kind: Kind) -> Self {
        Self {
            line_id,
            note_id,
            time,
            end_time,
            kind,
            x: 0.,
            state: State::default(),
        }
    }

    pub fn skip(&mut self) {
        self.state.skipped = true;
    }

    pub fn visual(&self, now: f64, speed: f64, good: f64) -> Visual {
        let state = &self.state;
        if state.skipped || state.outcome == Some(Outcome::Miss) {
            return Visual::Finished;
        }
        if self.kind == Kind::Hold && (state.clicked || state.head.is_some()) {
            if state.outcome.is_some() && now >= self.end_time {
                return Visual::Finished;
            }
            return Visual::Hold {
                perfect: state.head == Some(true),
                head_time: state.head_time,
                pending: state.head.is_none(),
                scored: state.outcome.is_some(),
                safe_frames: state.safe_frames,
            };
        }
        if state.outcome.is_some() {
            // Preserve the existing early-Bad sprite lifetime independently
            // of its final score and exclusion from matching.
            if state.outcome == Some(Outcome::Bad) && (self.time - now) / speed > good {
                return Visual::Armed;
            }
            return Visual::Finished;
        }
        if state.clicked || state.matched {
            Visual::Armed
        } else {
            Visual::Waiting
        }
    }
}

pub(crate) enum Visual {
    Waiting,
    Armed,
    Hold {
        perfect: bool,
        head_time: f64,
        pending: bool,
        scored: bool,
        safe_frames: i8,
    },
    Finished,
}

pub(crate) struct Finger {
    pub click: bool,
    pub flick: bool,
    /// Coordinates relative to each judge line, in world units.
    pub positions: Vec<Option<[f64; 2]>>,
}

pub(crate) struct Frame<'a> {
    pub now: f64,
    pub speed: f64,
    pub windows: Windows,
    pub fingers: &'a [Finger],
    pub held_positions: &'a [Vec<[f64; 2]>],
    pub keyboard_clicks: u32,
    pub keyboard_held: bool,
}

pub(crate) enum Event {
    Head { index: usize, perfect: bool },
    Final { index: usize, outcome: Outcome, hit_time: Option<f64> },
}

#[derive(Default)]
pub(crate) struct Result {
    pub events: Vec<Event>,
    pub consumed_fingers: Vec<usize>,
}

pub(crate) struct Judge {
    /// One stable global timeline, retaining scored notes for interval lookup.
    pub notes: Vec<Note>,
}

impl Judge {
    pub fn new(mut notes: Vec<Note>) -> Self {
        notes.sort_by(|a, b| {
            a.time
                .total_cmp(&b.time)
                .then_with(|| a.line_id.cmp(&b.line_id))
                .then_with(|| a.note_id.cmp(&b.note_id))
        });
        Self { notes }
    }

    pub fn reset(&mut self) {
        for note in &mut self.notes {
            note.state = State::default();
        }
    }

    fn interval(&self, frame: &Frame<'_>, early: f64, late: f64) -> std::ops::Range<usize> {
        let end = self.notes.partition_point(|note| (note.time - frame.now) / frame.speed < early);
        if end == 0 {
            return 0..0;
        }
        let mut start = end - 1;
        // Intentionally reproduce start=end when no normal candidate exists.
        // Filtering each note by a strict late limit would erase late Bad.
        while start > 0 && (self.notes[start - 1].time - frame.now) / frame.speed > -late {
            start -= 1;
        }
        start..end
    }

    fn candidate(&self, frame: &Frame<'_>, finger: &Finger, flick: bool, custom: Option<(f64, f64)>) -> Option<usize> {
        let windows = frame.windows;
        let (early, late, width) = if flick {
            (windows.flick, windows.flick, 2.1)
        } else {
            (windows.bad, windows.good, 1.9)
        };
        let mut best: Option<usize> = None;
        let mut best_abs_dt = 10000.;
        let mut best_metric = 10000.;
        let range = if let Some((early, miss)) = custom.filter(|_| flick) {
            let start = self.notes.partition_point(|note| (note.time - frame.now) / frame.speed <= miss);
            let end = self.notes.partition_point(|note| (note.time - frame.now) / frame.speed <= early);
            start..end.max(start)
        } else {
            self.interval(frame, early, late)
        };
        for index in range {
            let note = &self.notes[index];
            let state = &note.state;
            if state.skipped
                || if flick {
                    note.kind != Kind::Flick || state.matched
                } else {
                    state.clicked
                }
            {
                continue;
            }
            let dt = (note.time - frame.now) / frame.speed;
            if dt >= best_abs_dt + CANDIDATE_EPSILON {
                continue;
            }
            let Some([x, y]) = finger.positions.get(note.line_id).copied().flatten() else {
                continue;
            };
            let dx = (note.x - x).abs();
            if dx >= width {
                continue;
            }
            if !flick && dt > windows.bad - (dx - 0.9).max(0.) * windows.perfect * 0.5 {
                continue;
            }
            let metric = dx + (y / 2.2).abs();
            if let Some(best_index) = best {
                let previous = &self.notes[best_index];
                if flick || matches!(previous.kind, Kind::Tap | Kind::Hold) {
                    if !flick && !matches!(note.kind, Kind::Tap | Kind::Hold) {
                        continue;
                    }
                    if (note.time - previous.time).abs() / frame.speed > CANDIDATE_EPSILON || metric >= best_metric {
                        continue;
                    }
                }
            }
            best = Some(index);
            best_abs_dt = dt.abs();
            best_metric = metric;
        }
        best
    }

    pub fn step(&mut self, frame: Frame<'_>) -> Result {
        self.step_impl(frame, None)
    }

    /// Reuse the original gesture matching/arming/resolution, changing only
    /// early activation and unarmed Miss boundaries (signed seconds, early positive).
    pub fn step_custom_gestures(&mut self, frame: Frame<'_>, early: f64, miss: f64) -> Result {
        self.step_impl(frame, Some((early, miss)))
    }

    fn step_impl(&mut self, frame: Frame<'_>, custom: Option<(f64, f64)>) -> Result {
        let mut result = Result::default();
        // Match all gestures before scoring: a note crossing its late limit
        // this frame must still be available to the interval fallback.
        for finger in frame.fingers.iter().filter(|finger| finger.click) {
            if let Some(index) = self.candidate(&frame, finger, false, custom) {
                if self.notes[index].kind != Kind::Flick {
                    self.notes[index].state.clicked = true;
                }
            }
        }
        for (finger_id, finger) in frame.fingers.iter().enumerate().filter(|(_, finger)| finger.flick) {
            if let Some(index) = self.candidate(&frame, finger, true, custom) {
                self.notes[index].state.matched = true;
                result.consumed_fingers.push(finger_id);
            }
        }
        // Keyboard is an explicit desktop extension without spatial matching.
        for _ in 0..frame.keyboard_clicks {
            if let Some(note) = self.notes.iter_mut().find(|note| {
                !note.state.skipped
                    && !note.state.clicked
                    && matches!(note.kind, Kind::Tap | Kind::Hold)
                    && ((note.time - frame.now) / frame.speed).abs() < if note.kind == Kind::Tap { frame.windows.bad } else { frame.windows.good }
            }) {
                note.state.clicked = true;
            }
        }
        for (index, note) in self.notes.iter_mut().enumerate() {
            let state = &mut note.state;
            if state.skipped || state.outcome.is_some() {
                continue;
            }
            let dt = (note.time - frame.now) / frame.speed;
            let windows = frame.windows;
            let present = |width| {
                frame.keyboard_held
                    || frame
                        .held_positions
                        .get(note.line_id)
                        .is_some_and(|positions| positions.iter().any(|p| (note.x - p[0]).abs() < width))
            };
            let mut outcome = None;
            let mut hit_time = None;
            match note.kind {
                Kind::Tap => {
                    if state.clicked {
                        outcome = Some(if dt.abs() < windows.perfect {
                            Outcome::Perfect
                        } else if dt.abs() < windows.good {
                            Outcome::Good
                        } else {
                            Outcome::Bad
                        });
                        hit_time = Some(frame.now);
                    } else if dt < -windows.good {
                        outcome = Some(Outcome::Miss);
                    }
                }
                Kind::Hold => {
                    if state.head.is_none() {
                        if !state.clicked {
                            if dt < -windows.good {
                                outcome = Some(Outcome::Miss);
                            }
                        } else if dt.abs() < windows.good {
                            let perfect = dt.abs() < windows.perfect;
                            state.head = Some(perfect);
                            state.head_time = frame.now;
                            state.safe_frames = 2;
                            result.events.push(Event::Head { index, perfect });
                        }
                    }
                    if state.head.is_some() && outcome.is_none() {
                        if present(1.9) {
                            state.safe_frames = 2;
                        } else if state.safe_frames < 0 {
                            outcome = Some(Outcome::Miss);
                        } else {
                            state.safe_frames -= 1;
                        }
                        // Contact failure takes precedence on the tail frame.
                        if outcome.is_none() && (note.end_time - frame.now) / frame.speed < HOLD_TAIL {
                            outcome = Some(if state.head == Some(true) { Outcome::Perfect } else { Outcome::Good });
                            hit_time = Some(state.head_time);
                        }
                    }
                    if state.head.is_none() && outcome.is_none() && (frame.now - note.end_time) / frame.speed > HOLD_LATE_MISS {
                        outcome = Some(Outcome::Miss);
                    }
                }
                Kind::Drag => {
                    // A CheckNote click never arms DragControl.
                    if !state.matched && custom.map_or(dt.abs() <= DRAG_WINDOW, |(early, miss)| dt <= early && dt > miss) && present(2.1) {
                        state.matched = true;
                    }
                    if !state.matched && custom.map_or(dt < -DRAG_WINDOW, |(_, miss)| dt <= miss) {
                        outcome = Some(Outcome::Miss);
                    } else if state.matched && dt < RESOLVE_EARLY {
                        outcome = Some(Outcome::Perfect);
                    }
                }
                Kind::Flick => {
                    if !state.matched && custom.map_or(dt < -windows.flick, |(_, miss)| dt <= miss) {
                        outcome = Some(Outcome::Miss);
                    } else if state.matched && dt < RESOLVE_EARLY {
                        outcome = Some(Outcome::Perfect);
                    }
                }
            }
            if let Some(outcome) = outcome {
                state.outcome = Some(outcome);
                if note.kind != Kind::Hold {
                    state.clicked = true;
                }
                if note.kind == Kind::Flick {
                    state.matched = true;
                }
                result.events.push(Event::Final { index, outcome, hit_time });
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_gestures_only_override_outer_boundaries() {
        for kind in [Kind::Drag, Kind::Flick] {
            let held = vec![vec![[0., 0.]]];
            let input = [finger(false, kind == Kind::Flick, 0.)];
            let mut judge = Judge::new(vec![note(kind, 0.)]);
            assert!(judge.step_custom_gestures(frame(-0.181, &input, &held), 0.180, -0.165).events.is_empty());
            assert!(!judge.notes[0].state.matched);
            assert!(judge.step_custom_gestures(frame(-0.180, &input, &held), 0.180, -0.165).events.is_empty());
            assert!(judge.notes[0].state.matched);
            let result = judge.step_custom_gestures(frame(0., &[], &[]), 0.180, -0.165);
            assert!(matches!(
                result.events.as_slice(),
                [Event::Final {
                    outcome: Outcome::Perfect,
                    ..
                }]
            ));
            let mut late = Judge::new(vec![note(kind, 0.)]);
            let result = late.step_custom_gestures(frame(0.160, &input, &held), 0.180, -0.165);
            assert!(matches!(
                result.events.as_slice(),
                [Event::Final {
                    outcome: Outcome::Perfect,
                    ..
                }]
            ));
            let mut missed = Judge::new(vec![note(kind, 0.)]);
            let result = missed.step_custom_gestures(frame(0.165, &input, &held), 0.180, -0.165);
            assert!(matches!(result.events.as_slice(), [Event::Final { outcome: Outcome::Miss, .. }]));
        }
    }
    #[test]
    fn custom_shifted_gesture_windows_still_only_produce_perfect_or_miss() {
        for kind in [Kind::Drag, Kind::Flick] {
            let held = vec![vec![[0., 0.]]];
            let input = [finger(false, kind == Kind::Flick, 0.)];
            let mut judge = Judge::new(vec![note(kind, 0.)]);
            let result = judge.step_custom_gestures(frame(0.300, &input, &held), -0.200, -0.500);
            assert!(matches!(
                result.events.as_slice(),
                [Event::Final {
                    outcome: Outcome::Perfect,
                    ..
                }]
            ));
        }
    }
    const NORMAL: Windows = Windows {
        perfect: 0.08,
        good: 0.18,
        bad: 0.22,
        flick: 0.14,
    };

    fn note(kind: Kind, time: f64) -> Note {
        Note::new(0, 0, time, time + 1., kind)
    }

    fn finger(click: bool, flick: bool, x: f64) -> Finger {
        Finger {
            click,
            flick,
            positions: vec![Some([x, 0.])],
        }
    }

    fn frame<'a>(now: f64, fingers: &'a [Finger], held_positions: &'a [Vec<[f64; 2]>]) -> Frame<'a> {
        Frame {
            now,
            speed: 1.,
            windows: NORMAL,
            fingers,
            held_positions,
            keyboard_clicks: 0,
            keyboard_held: false,
        }
    }

    fn outcomes(result: Result) -> Vec<Outcome> {
        result
            .events
            .into_iter()
            .filter_map(|event| match event {
                Event::Final { index, outcome, hit_time } => {
                    let _ = (index, hit_time);
                    Some(outcome)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn early_drag_click_is_only_a_click_flag_and_can_still_miss() {
        let mut judge = Judge::new(vec![note(Kind::Drag, 0.)]);
        assert!(outcomes(judge.step(frame(-0.2, &[finger(true, false, 0.)], &[vec![[0., 0.]]]))).is_empty());
        assert!(judge.notes[0].state.clicked);
        assert!(!judge.notes[0].state.matched);
        assert!(outcomes(judge.step(frame(0., &[], &[]))).is_empty());
        assert_eq!(outcomes(judge.step(frame(0.101, &[], &[]))), vec![Outcome::Miss]);
    }

    #[test]
    fn early_clicked_drag_requires_later_held_contact() {
        let mut judge = Judge::new(vec![note(Kind::Drag, 0.)]);
        judge.step(frame(-0.2, &[finger(true, false, 0.)], &[]));
        judge.step(frame(-0.1, &[], &[vec![[0., 0.]]]));
        assert!(judge.notes[0].state.matched);
        assert_eq!(outcomes(judge.step(frame(0., &[], &[]))), vec![Outcome::Perfect]);
    }

    #[test]
    fn drag_includes_both_time_endpoints_but_excludes_spatial_endpoint() {
        for now in [-0.1, 0.1] {
            let mut judge = Judge::new(vec![note(Kind::Drag, 0.)]);
            judge.step(frame(now, &[], &[vec![[2.099, 0.]]]));
            assert!(judge.notes[0].state.matched);
        }
        let mut judge = Judge::new(vec![note(Kind::Drag, 0.)]);
        judge.step(frame(0., &[], &[vec![[2.1, 0.]]]));
        assert!(!judge.notes[0].state.matched);
    }

    #[test]
    fn one_held_finger_matches_many_drags() {
        let mut judge = Judge::new(vec![note(Kind::Drag, 0.), note(Kind::Drag, 0.001)]);
        assert_eq!(outcomes(judge.step(frame(0., &[], &[vec![[0., 0.]]]))), vec![Outcome::Perfect, Outcome::Perfect]);
    }

    #[test]
    fn drag_and_flick_resolve_only_below_five_ms() {
        for kind in [Kind::Drag, Kind::Flick] {
            let mut judge = Judge::new(vec![note(kind, 0.)]);
            judge.notes[0].state.matched = true;
            assert!(outcomes(judge.step(frame(-0.005, &[], &[]))).is_empty());
            assert_eq!(outcomes(judge.step(frame(-0.0049, &[], &[]))), vec![Outcome::Perfect]);
        }
    }

    #[test]
    fn armed_red_and_yellow_continue_protecting_clicks_until_scoring() {
        for kind in [Kind::Drag, Kind::Flick] {
            let mut later = note(Kind::Tap, 0.03);
            later.note_id = 1;
            let mut judge = Judge::new(vec![note(kind, 0.), later]);
            judge.notes[0].state.matched = true;
            assert!(outcomes(judge.step(frame(-0.05, &[finger(true, false, 0.)], &[]))).is_empty());
            assert!(!judge.notes[1].state.clicked);
            assert_eq!(judge.notes[0].state.clicked, kind == Kind::Drag);
        }
    }

    #[test]
    fn flick_click_does_not_mark_or_consume_the_red_note() {
        let mut judge = Judge::new(vec![note(Kind::Flick, 0.)]);
        let result = judge.step(frame(-0.05, &[finger(true, false, 0.)], &[]));
        assert!(result.events.is_empty());
        assert!(result.consumed_fingers.is_empty());
        assert!(!judge.notes[0].state.clicked);
        assert!(!judge.notes[0].state.matched);
    }

    #[test]
    fn exactly_ten_ms_early_protects_but_late_side_can_use_distance() {
        let mut first = note(Kind::Tap, 0.);
        first.x = 1.;
        let mut second = note(Kind::Hold, 0.01);
        second.note_id = 1;
        let mut judge = Judge::new(vec![first, second]);
        let input = [finger(true, false, 0.)];
        assert_eq!(judge.candidate(&frame(0., &input, &[]), &input[0], false, None), Some(0));
        assert_eq!(judge.candidate(&frame(0.02, &input, &[]), &input[0], false, None), Some(1));
        judge.step(frame(0., &input, &[]));
        assert!(judge.notes[0].state.clicked);
        assert!(!judge.notes[1].state.clicked);
    }

    #[test]
    fn nearby_tap_and_hold_beat_weak_candidates_and_use_weighted_distance() {
        let mut weak = note(Kind::Drag, 0.);
        weak.x = 0.;
        let mut strong = note(Kind::Tap, 0.005);
        strong.x = 1.5;
        strong.note_id = 1;
        let judge = Judge::new(vec![weak, strong]);
        let input = finger(true, false, 0.);
        assert_eq!(judge.candidate(&frame(0., &[], &[]), &input, false, None), Some(1));
        let mut a = note(Kind::Tap, 0.);
        a.x = 0.2;
        let mut b = Note::new(1, 0, 0., 1., Kind::Hold);
        b.x = 0.3;
        let judge = Judge::new(vec![a, b]);
        let input = Finger {
            click: true,
            flick: false,
            positions: vec![Some([0., 2.2]), Some([0., 0.])],
        };
        assert_eq!(judge.candidate(&frame(0., &[], &[]), &input, false, None), Some(1));
    }

    #[test]
    fn global_flick_order_is_independent_of_line_order() {
        let mut notes: Vec<_> = [(2, 1.000, 3.), (1, 1.008, 2.), (0, 1.016, 1.)]
            .into_iter()
            .map(|(line, time, metric)| {
                let mut n = Note::new(line, 0, time, time, Kind::Flick);
                n.x = 0.;
                // Use y for large metrics without leaving the horizontal region.
                (n, metric)
            })
            .collect();
        notes.reverse();
        let input = Finger {
            click: false,
            flick: true,
            positions: vec![Some([0., 2.2]), Some([0., 4.4]), Some([0., 6.6])],
        };
        let mut judge = Judge::new(notes.into_iter().map(|(n, _)| n).collect());
        assert_eq!(judge.candidate(&frame(0.99, &[], &[]), &input, true, None), Some(2));
        let result = judge.step(frame(0.99, &[input], &[]));
        assert_eq!(result.consumed_fingers, vec![0]);
        assert_eq!(judge.notes.iter().filter(|n| n.state.matched).count(), 1);
        assert!(judge.notes[2].state.matched);
    }

    #[test]
    fn flick_ten_ms_early_guard_and_horizontal_open_boundary() {
        let mut a = note(Kind::Flick, 0.);
        a.x = 1.;
        let mut b = note(Kind::Flick, 0.01);
        b.note_id = 1;
        let judge = Judge::new(vec![a, b]);
        let input = finger(false, true, 0.);
        assert_eq!(judge.candidate(&frame(0., &[], &[]), &input, true, None), Some(0));
        let judge = Judge::new(vec![note(Kind::Flick, 0.)]);
        assert_eq!(judge.candidate(&frame(0., &[], &[]), &finger(false, true, 2.1), true, None), None);
    }

    #[test]
    fn multiple_fingers_match_distinct_notes_before_any_scoring() {
        for kind in [Kind::Tap, Kind::Flick] {
            let mut second = note(kind, 0.);
            second.note_id = 1;
            let mut judge = Judge::new(vec![note(kind, 0.), second]);
            let fingers = [
                finger(kind == Kind::Tap, kind == Kind::Flick, 0.),
                finger(kind == Kind::Tap, kind == Kind::Flick, 0.),
            ];
            let result = judge.step(frame(0., &fingers, &[]));
            assert_eq!(outcomes(result), vec![Outcome::Perfect, Outcome::Perfect]);
        }
    }

    #[test]
    fn sparse_late_tap_is_bad_and_late_flick_is_perfect() {
        for (kind, now, expected) in [(Kind::Tap, 0.181, Outcome::Bad), (Kind::Flick, 0.141, Outcome::Perfect)] {
            let mut judge = Judge::new(vec![note(kind, 0.)]);
            let input = [finger(kind == Kind::Tap, kind == Kind::Flick, 0.)];
            assert_eq!(outcomes(judge.step(frame(now, &input, &[]))), vec![expected]);
            assert!(outcomes(judge.step(frame(now + 0.01, &input, &[]))).is_empty());
        }
    }

    #[test]
    fn other_notes_in_global_interval_prevent_late_fallback_even_when_scored() {
        for kind in [Kind::Tap, Kind::Flick] {
            let mut blocking = note(Kind::Tap, 0.2);
            blocking.note_id = 1;
            let mut judge = Judge::new(vec![note(kind, 0.), blocking]);
            judge.notes[1].state.clicked = true;
            judge.notes[1].state.outcome = Some(Outcome::Perfect);
            let result = judge.step(frame(0.19, &[finger(kind == Kind::Tap, kind == Kind::Flick, 0.)], &[]));
            assert_eq!(outcomes(result), vec![Outcome::Miss]);
        }
    }

    #[test]
    fn already_missed_notes_cannot_be_revived() {
        for kind in [Kind::Tap, Kind::Hold, Kind::Flick] {
            let mut judge = Judge::new(vec![note(kind, 0.)]);
            assert_eq!(outcomes(judge.step(frame(0.2, &[], &[]))), vec![Outcome::Miss]);
            assert!(outcomes(judge.step(frame(0.21, &[finger(true, true, 0.)], &[]))).is_empty());
        }
    }

    #[test]
    fn late_hold_waits_until_tail_plus_250ms_without_head_or_contact_checks() {
        let mut judge = Judge::new(vec![note(Kind::Hold, 0.)]);
        assert!(judge.step(frame(0.181, &[finger(true, false, 0.)], &[])).events.is_empty());
        for now in [0.2, 0.4, 0.8, 1., 1.25] {
            assert!(judge.step(frame(now, &[], &[])).events.is_empty());
        }
        assert!(judge.notes[0].state.head.is_none());
        assert_eq!(outcomes(judge.step(frame(1.251, &[], &[]))), vec![Outcome::Miss]);
    }

    #[test]
    fn early_hold_waiting_does_not_spend_release_protection() {
        let mut judge = Judge::new(vec![note(Kind::Hold, 0.)]);
        judge.step(frame(-0.21, &[finger(true, false, 0.)], &[]));
        for now in [-0.205, -0.2, -0.195, -0.19, -0.185, -0.18] {
            assert!(judge.step(frame(now, &[], &[])).events.is_empty());
            assert_eq!(judge.notes[0].state.safe_frames, 2);
        }
        let result = judge.step(frame(-0.179, &[], &[]));
        assert!(matches!(result.events.as_slice(), [Event::Head { perfect: false, .. }]));
        assert_eq!(judge.notes[0].state.safe_frames, 1);
        assert!(outcomes(judge.step(frame(-0.17, &[], &[]))).is_empty());
        assert!(outcomes(judge.step(frame(-0.16, &[], &[]))).is_empty());
        assert_eq!(outcomes(judge.step(frame(-0.15, &[], &[]))), vec![Outcome::Miss]);
    }

    #[test]
    fn delayed_hold_head_can_be_perfect_after_a_frame_jump() {
        let mut judge = Judge::new(vec![note(Kind::Hold, 0.)]);
        judge.step(frame(-0.21, &[finger(true, false, 0.)], &[]));
        let result = judge.step(frame(-0.05, &[], &[vec![[0., 0.]]]));
        assert!(matches!(result.events.as_slice(), [Event::Head { index: 0, perfect: true }]));
    }

    #[test]
    fn jumping_past_entire_hold_head_window_does_not_award_good() {
        let mut judge = Judge::new(vec![note(Kind::Hold, 0.)]);
        judge.step(frame(-0.21, &[finger(true, false, 0.)], &[]));
        assert!(judge.step(frame(0.19, &[], &[vec![[0., 0.]]])).events.is_empty());
        assert!(judge.notes[0].state.head.is_none());
        assert_eq!(outcomes(judge.step(frame(1.251, &[], &[]))), vec![Outcome::Miss]);
    }

    #[test]
    fn hold_fourth_missing_frame_misses_before_tail_scoring() {
        let mut n = note(Kind::Hold, 0.);
        n.end_time = 0.5;
        let mut judge = Judge::new(vec![n]);
        judge.step(frame(0., &[finger(true, false, 0.)], &[vec![[0., 0.]]]));
        for now in [0.24, 0.26, 0.279] {
            assert!(outcomes(judge.step(frame(now, &[], &[]))).is_empty());
        }
        assert_eq!(outcomes(judge.step(frame(0.29, &[], &[]))), vec![Outcome::Miss]);
    }

    #[test]
    fn hold_tail_endpoint_is_open_and_success_keeps_tail_visual() {
        let mut n = note(Kind::Hold, 0.);
        n.end_time = 0.22;
        let mut judge = Judge::new(vec![n]);
        let result = judge.step(frame(0., &[finger(true, false, 0.)], &[vec![[0., 0.]]]));
        assert!(matches!(result.events.as_slice(), [Event::Head { perfect: true, .. }]));
        assert_eq!(outcomes(judge.step(frame(0.001, &[], &[]))), vec![Outcome::Perfect]);
        assert!(matches!(
            judge.notes[0].visual(0.01, 1., NORMAL.good),
            Visual::Hold {
                scored: true,
                perfect: true,
                pending: false,
                ..
            }
        ));
        assert!(matches!(judge.notes[0].visual(0.22, 1., NORMAL.good), Visual::Finished));
        assert!(judge.step(frame(0.3, &[], &[])).events.is_empty());
    }

    #[test]
    fn release_protection_recovers_with_a_different_finger() {
        let mut judge = Judge::new(vec![note(Kind::Hold, 0.)]);
        judge.step(frame(0., &[finger(true, false, 0.)], &[vec![[0., 0.]]]));
        for now in [0.01, 0.02, 0.03] {
            assert!(outcomes(judge.step(frame(now, &[], &[]))).is_empty());
        }
        assert!(outcomes(judge.step(frame(0.04, &[], &[vec![[0., 0.]]]))).is_empty());
        assert_eq!(judge.notes[0].state.safe_frames, 2);
    }

    #[test]
    fn tap_and_flick_early_endpoints_are_open() {
        for (kind, now) in [(Kind::Tap, -0.22), (Kind::Flick, -0.14)] {
            let mut judge = Judge::new(vec![note(kind, 0.)]);
            let result = judge.step(frame(now, &[finger(kind == Kind::Tap, kind == Kind::Flick, 0.)], &[]));
            assert!(result.events.is_empty());
            assert!(!judge.notes[0].state.clicked);
            assert!(!judge.notes[0].state.matched);
        }
    }

    #[test]
    fn tap_perfect_and_good_endpoints_are_open() {
        for (now, expected) in [
            (-0.0799, Outcome::Perfect),
            (-0.08, Outcome::Good),
            (-0.1799, Outcome::Good),
            (-0.18, Outcome::Bad),
            (0.18, Outcome::Bad),
        ] {
            let mut judge = Judge::new(vec![note(Kind::Tap, 0.)]);
            assert_eq!(outcomes(judge.step(frame(now, &[finger(true, false, 0.)], &[]))), vec![expected]);
        }
    }

    #[test]
    fn click_width_and_bad_shrink_follow_world_units() {
        let judge = Judge::new(vec![note(Kind::Tap, 0.)]);
        let f = frame(-0.2, &[], &[]);
        assert_eq!(judge.candidate(&f, &finger(true, false, 0.9), false, None), Some(0));
        assert_eq!(judge.candidate(&f, &finger(true, false, 1.5), false, None), None);
        assert_eq!(judge.candidate(&frame(0., &[], &[]), &finger(true, false, 1.9), false, None), None);
    }

    #[test]
    fn real_time_windows_and_ten_ms_rule_survive_practice_speed_changes() {
        for speed in [0.05, 0.5, 1., 2., 10.] {
            let mut drag = note(Kind::Drag, 0.);
            let mut tap = note(Kind::Tap, 0.02 * speed);
            drag.note_id = 0;
            tap.note_id = 1;
            let judge = Judge::new(vec![drag, tap]);
            let mut f = frame(-0.05 * speed, &[], &[]);
            f.speed = speed;
            assert_eq!(judge.candidate(&f, &finger(true, false, 0.), false, None), Some(0));
        }
    }

    #[test]
    fn screen_height_world_unit_is_independent_of_viewport_aspect() {
        for (width, height) in [(1920., 1080.), (2400., 1080.), (1440., 1080.)] {
            let unit = world_unit(height, width);
            assert!((unit * width / 2. - height / 10.).abs() < 1e-9);
        }
        assert!((world_unit(1080., 1920.) - 2. * 9. / 160.).abs() < 1e-9);
    }

    #[test]
    fn small_previous_flick_displacement_always_resets_relative_speed() {
        let (relative, current) = flick_speeds([0.09, 0.], [0.09, 0.], 1. / 120.);
        assert_eq!(relative, 0.);
        assert!(current >= 5. * 0.06 * 200. / 380.);
        assert_eq!(flick_speeds([0.1, 0.], [0.5, 0.], 1. / 60.).0, 0.);
        assert_eq!(flick_speeds([0., 0.], [0., 0.], 1. / 60.), (0., 0.));
    }

    #[test]
    fn flick_projection_is_directional_and_frame_rate_scaled() {
        let (same, speed) = flick_speeds([0.5, 0.], [0.4, 0.], 1. / 60.);
        let (reverse, _) = flick_speeds([0.5, 0.], [-0.4, 0.], 1. / 60.);
        assert!((same - 0.4).abs() < 1e-6);
        assert!((speed - 0.4).abs() < 1e-6);
        assert!((reverse + 0.4).abs() < 1e-6);
        assert!(flick_speeds([0.5, 0.], [0.4, 0.], 1. / 120.).0 > same);
    }

    #[test]
    fn reset_and_exercise_skip_do_not_revive_or_duplicate_notes() {
        let mut judge = Judge::new(vec![note(Kind::Tap, 0.)]);
        judge.notes[0].skip();
        assert!(judge.step(frame(0., &[finger(true, false, 0.)], &[])).events.is_empty());
        judge.reset();
        assert!(matches!(judge.notes[0].visual(-0.2, 1., NORMAL.good), Visual::Waiting));
        assert_eq!(outcomes(judge.step(frame(0., &[finger(true, false, 0.)], &[]))), vec![Outcome::Perfect]);
        assert!(judge.step(frame(0., &[finger(true, false, 0.)], &[])).events.is_empty());
    }

    #[test]
    fn keyboard_extension_marks_heads_and_holds_drag_contact() {
        let mut judge = Judge::new(vec![note(Kind::Hold, 0.)]);
        let mut f = frame(0., &[], &[]);
        f.keyboard_clicks = 1;
        f.keyboard_held = true;
        assert!(matches!(judge.step(f).events.as_slice(), [Event::Head { perfect: true, .. }]));
        let mut judge = Judge::new(vec![note(Kind::Drag, 0.)]);
        let mut f = frame(0., &[], &[]);
        f.keyboard_held = true;
        assert_eq!(outcomes(judge.step(f)), vec![Outcome::Perfect]);
    }

    #[test]
    fn head_event_and_final_report_preserve_original_head_time() {
        let mut judge = Judge::new(vec![note(Kind::Hold, 0.)]);
        let result = judge.step(frame(0.04, &[finger(true, false, 0.)], &[vec![[0., 0.]]]));
        assert!(matches!(result.events.as_slice(), [Event::Head { index: 0, perfect: true }]));
        match judge.notes[0].visual(0.04, 1., NORMAL.good) {
            Visual::Hold {
                head_time,
                safe_frames,
                scored,
                ..
            } => {
                assert_eq!(head_time, 0.04);
                assert_eq!(safe_frames, 2);
                assert!(!scored);
            }
            _ => panic!("successful head must enter the Hold renderer state"),
        }
        let result = judge.step(frame(0.79, &[], &[]));
        assert!(matches!(result.events.as_slice(), [Event::Final { index: 0, outcome: Outcome::Perfect, hit_time: Some(time) }] if *time == 0.04));
    }
}
