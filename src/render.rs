//! Turns a [`Diff`] into plain English.
//!
//! Kept separate from the differ so the structured diff stays usable on its
//! own. Lines are written the way an editor would describe the edit, so
//! positions read as timecode and effect parameters use the names shown in
//! Kdenlive's UI where we know them.

use crate::differ::{
    BinChange, ClipChange, ClipChangeKind, Diff, GuideChange, MarkerChange, ParamChange,
    TrackChange,
};
use crate::model::Frames;
use crate::xml_parser::format_duration;

/// How many parameter changes to name before summarizing the rest. A colour
/// wheel effect can touch a dozen parameters in one drag, and listing all of
/// them buries the useful part of the line.
const MAX_NAMED_PARAMS: usize = 3;

pub fn render(diff: &Diff, fps: f64) -> Vec<String> {
    let mut lines = Vec::new();
    let at = |f: Frames| format_duration(f, fps);

    for change in &diff.profile_changes {
        lines.push(format!(
            "changed project {} from {} to {}",
            change.what, change.from, change.to
        ));
    }

    for change in &diff.track_changes {
        lines.push(match change {
            TrackChange::Added { track } => format!("added track {track}"),
            TrackChange::Removed { track } => format!("removed track {track}"),
            TrackChange::Locked { track } => format!("locked track {track}"),
            TrackChange::Unlocked { track } => format!("unlocked track {track}"),
        });
    }

    for change in &diff.bin_changes {
        lines.push(match change {
            BinChange::Added { clip } => format!("added {clip} to the project bin"),
            BinChange::Removed { clip } => format!("removed {clip} from the project bin"),
            BinChange::Renamed { from, to } => format!("renamed bin clip {from} to {to}"),
        });
    }

    for change in &diff.clip_changes {
        lines.push(render_clip_change(change, &at));
    }

    for change in &diff.marker_changes {
        lines.push(match change {
            MarkerChange::Added { position, comment } => {
                format!("added marker {} at {}", quoted(comment), at(*position))
            }
            MarkerChange::Removed { position, comment } => {
                format!("removed marker {} at {}", quoted(comment), at(*position))
            }
            MarkerChange::Moved { from, to, comment } => format!(
                "moved marker {} from {} to {}",
                quoted(comment),
                at(*from),
                at(*to)
            ),
            MarkerChange::Retitled { position, from, to } => format!(
                "renamed marker at {} from {} to {}",
                at(*position),
                quoted(from),
                quoted(to)
            ),
        });
    }

    for change in &diff.guide_changes {
        lines.push(match change {
            GuideChange::Added { position, comment } => {
                format!("added guide {} at {}", quoted(comment), at(*position))
            }
            GuideChange::Removed { position, comment } => {
                format!("removed guide {} at {}", quoted(comment), at(*position))
            }
            GuideChange::Retitled { position, from, to } => format!(
                "renamed guide at {} from {} to {}",
                at(*position),
                quoted(from),
                quoted(to)
            ),
        });
    }

    lines
}

fn render_clip_change(change: &ClipChange, at: &impl Fn(Frames) -> String) -> String {
    let (clip, track) = (&change.clip, &change.track);
    match &change.kind {
        ClipChangeKind::Added { position, duration } => format!(
            "added {clip} to {track} at {}, {} long",
            at(*position),
            at(*duration)
        ),
        ClipChangeKind::Removed { position } => {
            format!("removed {clip} from {track} at {}", at(*position))
        }
        ClipChangeKind::Moved { from, to } => {
            format!("moved {clip} on {track} from {} to {}", at(*from), at(*to))
        }
        ClipChangeKind::Trimmed {
            from_duration,
            to_duration,
            position,
        } => {
            let verb = if to_duration > from_duration {
                "extended"
            } else {
                "trimmed"
            };
            format!(
                "{verb} {clip} on {track} at {}, {} to {}",
                at(*position),
                at(*from_duration),
                at(*to_duration)
            )
        }
        ClipChangeKind::EffectAdded { effect } => {
            format!("added {} effect to {clip} on {track}", pretty(effect))
        }
        ClipChangeKind::EffectRemoved { effect } => {
            format!("removed {} effect from {clip} on {track}", pretty(effect))
        }
        ClipChangeKind::EffectDisabled { effect } => {
            format!("disabled {} effect on {clip}, {track}", pretty(effect))
        }
        ClipChangeKind::EffectEnabled { effect } => {
            format!("enabled {} effect on {clip}, {track}", pretty(effect))
        }
        ClipChangeKind::EffectChanged { effect, params } => {
            // Effects often share a name with their only parameter (gain, for
            // one), and "changed gain on gain effect" reads badly.
            let effect = pretty(effect);
            let single_named_param = params.len() == 1 && pretty(&params[0].name) == effect;
            if single_named_param {
                format!("changed {} on {clip}, {track}", render_params(params))
            } else {
                format!(
                    "changed {} on {effect} effect, {clip} on {track}",
                    render_params(params)
                )
            }
        }
    }
}

fn render_params(params: &[ParamChange]) -> String {
    let named: Vec<String> = params
        .iter()
        .take(MAX_NAMED_PARAMS)
        .map(|p| {
            // Keyframed values are a whole curve, and quoting both sides of one
            // is unreadable. Say the shape changed and leave the numbers out.
            if is_keyframed(&p.from) || is_keyframed(&p.to) {
                format!("{} keyframes", pretty(&p.name))
            } else if p.from.is_empty() {
                format!("{} to {}", pretty(&p.name), p.to)
            } else {
                format!("{} {} to {}", pretty(&p.name), p.from, p.to)
            }
        })
        .collect();

    let rest = params.len().saturating_sub(MAX_NAMED_PARAMS);
    match rest {
        0 => join(&named),
        1 => format!("{} and 1 other parameter", join(&named)),
        n => format!("{} and {n} other parameters", join(&named)),
    }
}

/// MLT writes keyframed values as `time=value;time=value`.
fn is_keyframed(value: &str) -> bool {
    value.contains('=') && value.contains(';')
}

fn join(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

/// Kdenlive effect and parameter ids are machine names like `lift_gamma_gain`
/// or `alpha_operation`. Underscores to spaces is enough to make them read.
fn pretty(name: &str) -> String {
    name.replace('_', " ")
}

fn quoted(text: &str) -> String {
    if text.is_empty() {
        "(no text)".to_string()
    } else {
        format!("\"{text}\"")
    }
}

/// One line summarizing a whole diff, for use as a commit subject.
pub fn summarize(diff: &Diff, fps: f64) -> String {
    let lines = render(diff, fps);
    match lines.len() {
        0 => "saved with no detected changes".to_string(),
        1 => capitalize(&lines[0]),
        n => {
            // Lead with the first change so the log stays scannable, and count
            // the rest rather than truncating mid sentence.
            format!(
                "{} and {} more change{}",
                capitalize(&lines[0]),
                n - 1,
                if n == 2 { "" } else { "s" }
            )
        }
    }
}

fn capitalize(line: &str) -> String {
    let mut chars = line.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
