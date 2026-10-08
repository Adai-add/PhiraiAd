//! Chart parsers
prpr_l10n::tl_file!("parser" ptl);

mod extra;
pub use extra::parse_extra;

mod pec;
pub use pec::parse_pec;

mod pgr;
pub use pgr::parse_phigros;

mod rpe;
pub use rpe::{lint, parse_rpe, RPE_HEIGHT, RPE_WIDTH};

#[derive(Debug, Default)]
pub struct ParseWarnings {
    pub has_new_speed_events: bool,
    pub has_attach_ui: bool,
}

pub(crate) fn process_lines(v: &mut [crate::core::JudgeLine]) {
    process_lines_with_real_hints(v, false);
}

/// Generated pure charts retain visual fake notes at their exact original times.
/// Only their playable copies contribute to multi-note hints.
pub(crate) fn process_lines_with_real_hints(v: &mut [crate::core::JudgeLine], real_only: bool) {
    use crate::ext::NotNanExt;
    let mut times = Vec::new();
    // TODO optimize using k-merge sort
    let sorts = v
        .iter()
        .map(|line| {
            let mut idx: Vec<usize> = (0..line.notes.len()).filter(|i| !real_only || !line.notes[*i].fake).collect();
            idx.sort_by_key(|id| line.notes[*id].time.not_nan());
            idx
        })
        .collect::<Vec<_>>();
    for (line, idx) in v.iter_mut().zip(sorts.iter()) {
        let v = &mut line.notes;
        let mut i = 0;
        while i < idx.len() {
            times.push(v[idx[i]].time.not_nan());
            let mut j = i + 1;
            while j < idx.len() && v[idx[j]].time == v[idx[i]].time {
                j += 1;
            }
            if j != i + 1 {
                times.push(v[idx[i]].time.not_nan());
            }
            i = j;
        }
    }
    times.sort();
    let mut mt = Vec::new();
    if !times.is_empty() {
        for i in 0..(times.len() - 1) {
            // since times are generated in the same way, theoretically we can compare them directly
            if times[i] == times[i + 1] && (i == 0 || times[i - 1] != times[i]) {
                mt.push(*times[i]);
            }
        }
    }
    for (line, idx) in v.iter_mut().zip(sorts.iter()) {
        let mut i = 0;
        for id in idx {
            let note = &mut line.notes[*id];
            let time = note.time;
            while i < mt.len() && mt[i] < time {
                i += 1;
            }
            if i < mt.len() && mt[i] == time {
                note.multiple_hint = true;
            }
        }
    }
}

#[cfg(test)]
mod pure_config_tests {
    use super::*;

    #[test]
    fn fake_copies_do_not_turn_single_notes_into_multi_notes() {
        let notes = [32, 32, 64, 64].map(|time| serde_json::json!({"type":1,"time":time,"positionX":0,"holdTime":0,"speed":1,"floorPosition":0}));
        let source = serde_json::json!({"formatVersion":3,"offset":0,"judgeLineList":[{"bpm":120,"notesAbove":notes,"notesBelow":[],"speedEvents":[{"startTime":0,"endTime":128,"value":1}],"judgeLineDisappearEvents":[{"startTime":0,"endTime":128,"start":1,"end":1}],"judgeLineRotateEvents":[{"startTime":0,"endTime":128,"start":0,"end":0}],"judgeLineMoveEvents":[{"startTime":0,"endTime":128,"start":0.5,"end":0.5,"start2":0.5,"end2":0.5}]}]});
        let mut chart = pgr::parse_phigros(&source.to_string(), Default::default()).unwrap();
        assert!(chart.lines[0].notes.iter().all(|n| n.multiple_hint));
        for note in &mut chart.lines[0].notes {
            note.multiple_hint = false;
        }
        chart.lines[0].notes[0].fake = true;
        process_lines_with_real_hints(&mut chart.lines, true);
        let notes = &chart.lines[0].notes;
        assert!(!notes[0].multiple_hint);
        assert!(!notes[1].multiple_hint);
        assert!(notes[2].multiple_hint && notes[3].multiple_hint);
    }
}

#[rustfmt::skip]
pub const RPE_TWEEN_MAP: [crate::core::TweenId; 30] = {
    use crate::core::{easing_from as e, TweenMajor::*, TweenMinor::*};
    [
        2, 2, // linear
        e(Sine, Out), e(Sine, In),
        e(Quad, Out), e(Quad, In),
        e(Sine, InOut), e(Quad, InOut),
        e(Cubic, Out), e(Cubic, In),
        e(Quart, Out), e(Quart, In),
        e(Cubic, InOut), e(Quart, InOut),
        e(Quint, Out), e(Quint, In),
        e(Expo, Out), e(Expo, In),
        e(Circ, Out), e(Circ, In),
        e(Back, Out), e(Back, In),
        e(Circ, InOut), e(Back, InOut),
        e(Elastic, Out), e(Elastic, In),
        e(Bounce, Out), e(Bounce, In),
        e(Bounce, InOut), e(Elastic, InOut),
    ]
};
