//! Compares two parsed projects and reports what changed as structured data.
//!
//! The output is deliberately Rust values rather than text. `render` turns it
//! into sentences, and keeping the two apart means the same diff can drive
//! something else later without reparsing prose.
//!
//! The hard part is identity. Kdenlive renumbers producers between saves, so
//! nothing in the file reliably says "this is the same clip as last time".
//! Clips are matched per track by what they play and where they sit, which is
//! what lets a move read as a move instead of a delete plus an add.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{BinClip, Effect, Frames, Guide, Marker, Project, TimelineClip, Track};

#[derive(Debug, Clone, PartialEq)]
pub struct Diff {
    pub profile_changes: Vec<ProfileChange>,
    pub bin_changes: Vec<BinChange>,
    pub track_changes: Vec<TrackChange>,
    pub clip_changes: Vec<ClipChange>,
    pub marker_changes: Vec<MarkerChange>,
    pub guide_changes: Vec<GuideChange>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.profile_changes.is_empty()
            && self.bin_changes.is_empty()
            && self.track_changes.is_empty()
            && self.clip_changes.is_empty()
            && self.marker_changes.is_empty()
            && self.guide_changes.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProfileChange {
    pub what: &'static str,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinChange {
    Added { clip: String },
    Removed { clip: String },
    Renamed { from: String, to: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrackChange {
    Added { track: String },
    Removed { track: String },
    Locked { track: String },
    Unlocked { track: String },
}

/// A change to one clip on the timeline. `track` and `clip` are display names
/// so that rendering never has to look anything up again.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipChange {
    pub track: String,
    pub clip: String,
    pub kind: ClipChangeKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClipChangeKind {
    Added {
        position: Frames,
        duration: Frames,
    },
    Removed {
        position: Frames,
    },
    Moved {
        from: Frames,
        to: Frames,
    },
    /// In or out point changed, so the clip plays a different span of source.
    Trimmed {
        from_duration: Frames,
        to_duration: Frames,
        position: Frames,
    },
    EffectAdded {
        effect: String,
    },
    EffectRemoved {
        effect: String,
    },
    EffectChanged {
        effect: String,
        params: Vec<ParamChange>,
    },
    EffectDisabled {
        effect: String,
    },
    EffectEnabled {
        effect: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamChange {
    pub name: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MarkerChange {
    Added {
        position: Frames,
        comment: String,
    },
    Removed {
        position: Frames,
        comment: String,
    },
    Moved {
        from: Frames,
        to: Frames,
        comment: String,
    },
    Retitled {
        position: Frames,
        from: String,
        to: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum GuideChange {
    Added {
        position: Frames,
        comment: String,
    },
    Removed {
        position: Frames,
        comment: String,
    },
    Retitled {
        position: Frames,
        from: String,
        to: String,
    },
}

pub fn diff(before: &Project, after: &Project) -> Diff {
    Diff {
        profile_changes: diff_profile(before, after),
        bin_changes: diff_bin(before, after),
        track_changes: diff_tracks(before, after),
        clip_changes: diff_clips(before, after),
        marker_changes: diff_markers(before, after),
        guide_changes: diff_guides(before, after),
    }
}

fn diff_profile(before: &Project, after: &Project) -> Vec<ProfileChange> {
    let mut out = Vec::new();
    let (a, b) = (&before.profile, &after.profile);

    if (a.width, a.height) != (b.width, b.height) {
        out.push(ProfileChange {
            what: "resolution",
            from: format!("{}x{}", a.width, a.height),
            to: format!("{}x{}", b.width, b.height),
        });
    }
    if (a.frame_rate_num, a.frame_rate_den) != (b.frame_rate_num, b.frame_rate_den) {
        out.push(ProfileChange {
            what: "frame rate",
            from: format_fps(a.fps()),
            to: format_fps(b.fps()),
        });
    }
    out
}

fn format_fps(fps: f64) -> String {
    if (fps.round() - fps).abs() < 0.001 {
        format!("{fps:.0} fps")
    } else {
        format!("{fps:.2} fps")
    }
}

fn diff_bin(before: &Project, after: &Project) -> Vec<BinChange> {
    let mut out = Vec::new();

    for (id, old) in &before.bin_clips {
        match after.bin_clips.get(id) {
            None => out.push(BinChange::Removed { clip: old.label() }),
            Some(new) if old.label() != new.label() => out.push(BinChange::Renamed {
                from: old.label(),
                to: new.label(),
            }),
            Some(_) => {}
        }
    }
    for (id, new) in &after.bin_clips {
        if !before.bin_clips.contains_key(id) {
            out.push(BinChange::Added { clip: new.label() });
        }
    }
    out
}

fn diff_tracks(before: &Project, after: &Project) -> Vec<TrackChange> {
    let mut out = Vec::new();
    let old: BTreeMap<&str, &Track> = tracks_by_name(before);
    let new: BTreeMap<&str, &Track> = tracks_by_name(after);

    for (name, old_track) in &old {
        match new.get(name) {
            None => out.push(TrackChange::Removed {
                track: (*name).to_string(),
            }),
            Some(new_track) => {
                if !old_track.locked && new_track.locked {
                    out.push(TrackChange::Locked {
                        track: (*name).to_string(),
                    });
                } else if old_track.locked && !new_track.locked {
                    out.push(TrackChange::Unlocked {
                        track: (*name).to_string(),
                    });
                }
            }
        }
    }
    for name in new.keys() {
        if !old.contains_key(name) {
            out.push(TrackChange::Added {
                track: (*name).to_string(),
            });
        }
    }
    out
}

fn tracks_by_name(project: &Project) -> BTreeMap<&str, &Track> {
    project
        .sequences
        .iter()
        .flat_map(|s| &s.tracks)
        .map(|t| (t.name.as_str(), t))
        .collect()
}

fn diff_clips(before: &Project, after: &Project) -> Vec<ClipChange> {
    let mut out = Vec::new();
    let old_tracks = tracks_by_name(before);
    let new_tracks = tracks_by_name(after);

    for (name, new_track) in &new_tracks {
        let Some(old_track) = old_tracks.get(name) else {
            continue; // A new track's clips are covered by the track add itself.
        };
        diff_track_clips(name, old_track, new_track, before, after, &mut out);
    }
    out
}

fn diff_track_clips(
    track: &str,
    old_track: &Track,
    new_track: &Track,
    before: &Project,
    after: &Project,
    out: &mut Vec<ClipChange>,
) {
    let pairs = match_clips(&old_track.clips, &new_track.clips);
    let label = |project: &Project, clip: &TimelineClip| -> String {
        project
            .bin_clips
            .get(&clip.bin_id)
            .map(BinClip::label)
            .unwrap_or_else(|| format!("clip {}", clip.bin_id))
    };

    for pair in pairs {
        match pair {
            Match::Both(old, new) => {
                let name = label(after, new);
                if old.position != new.position {
                    out.push(ClipChange {
                        track: track.to_string(),
                        clip: name.clone(),
                        kind: ClipChangeKind::Moved {
                            from: old.position,
                            to: new.position,
                        },
                    });
                }
                if (old.source_in, old.source_out) != (new.source_in, new.source_out) {
                    out.push(ClipChange {
                        track: track.to_string(),
                        clip: name.clone(),
                        kind: ClipChangeKind::Trimmed {
                            from_duration: old.duration(),
                            to_duration: new.duration(),
                            position: new.position,
                        },
                    });
                }
                diff_effects(track, &name, old, new, out);
            }
            Match::Removed(old) => out.push(ClipChange {
                track: track.to_string(),
                clip: label(before, old),
                kind: ClipChangeKind::Removed {
                    position: old.position,
                },
            }),
            Match::Added(new) => out.push(ClipChange {
                track: track.to_string(),
                clip: label(after, new),
                kind: ClipChangeKind::Added {
                    position: new.position,
                    duration: new.duration(),
                },
            }),
        }
    }
}

enum Match<'a> {
    Both(&'a TimelineClip, &'a TimelineClip),
    Removed(&'a TimelineClip),
    Added(&'a TimelineClip),
}

/// Pairs up clips on one track between two saves.
///
/// Only clips playing the same bin clip can pair, since a different source is
/// a different clip no matter where it sits. Within a source, the closest
/// remaining pair by position wins, so nudging one clip does not cascade into
/// every later clip on the track looking changed.
fn match_clips<'a>(old: &'a [TimelineClip], new: &'a [TimelineClip]) -> Vec<Match<'a>> {
    let mut out = Vec::new();
    let mut used_new: BTreeSet<usize> = BTreeSet::new();
    let mut matched_old: BTreeSet<usize> = BTreeSet::new();

    // Exact position matches first. They are unambiguous, and taking them up
    // front stops a nearby clip from stealing the pairing.
    for (i, o) in old.iter().enumerate() {
        if let Some((j, _)) = new.iter().enumerate().find(|(j, n)| {
            !used_new.contains(j) && n.bin_id == o.bin_id && n.position == o.position
        }) {
            used_new.insert(j);
            matched_old.insert(i);
            out.push(Match::Both(o, &new[j]));
        }
    }

    // Then the leftovers, nearest position wins.
    for (i, o) in old.iter().enumerate() {
        if matched_old.contains(&i) {
            continue;
        }
        let best = new
            .iter()
            .enumerate()
            .filter(|(j, n)| !used_new.contains(j) && n.bin_id == o.bin_id)
            .min_by_key(|(_, n)| (n.position - o.position).abs());

        match best {
            Some((j, n)) => {
                used_new.insert(j);
                matched_old.insert(i);
                out.push(Match::Both(o, n));
            }
            None => out.push(Match::Removed(o)),
        }
    }

    for (j, n) in new.iter().enumerate() {
        if !used_new.contains(&j) {
            out.push(Match::Added(n));
        }
    }

    out
}

fn diff_effects(
    track: &str,
    clip: &str,
    old: &TimelineClip,
    new: &TimelineClip,
    out: &mut Vec<ClipChange>,
) {
    let change = |kind| ClipChange {
        track: track.to_string(),
        clip: clip.to_string(),
        kind,
    };

    // Effects are ordered on a clip and the same effect can appear twice, so
    // pair them by position within each service rather than by name alone.
    let mut old_by_name: BTreeMap<&str, Vec<&Effect>> = BTreeMap::new();
    let mut new_by_name: BTreeMap<&str, Vec<&Effect>> = BTreeMap::new();
    for e in &old.effects {
        old_by_name.entry(e.name.as_str()).or_default().push(e);
    }
    for e in &new.effects {
        new_by_name.entry(e.name.as_str()).or_default().push(e);
    }

    let names: BTreeSet<&str> = old_by_name
        .keys()
        .chain(new_by_name.keys())
        .copied()
        .collect();

    for name in names {
        let olds = old_by_name.get(name).map(Vec::as_slice).unwrap_or_default();
        let news = new_by_name.get(name).map(Vec::as_slice).unwrap_or_default();

        for pair in olds.iter().zip(news.iter()) {
            let (o, n) = pair;
            if o.disabled != n.disabled {
                out.push(change(if n.disabled {
                    ClipChangeKind::EffectDisabled {
                        effect: name.to_string(),
                    }
                } else {
                    ClipChangeKind::EffectEnabled {
                        effect: name.to_string(),
                    }
                }));
            }
            let params = diff_params(o, n);
            if !params.is_empty() {
                out.push(change(ClipChangeKind::EffectChanged {
                    effect: name.to_string(),
                    params,
                }));
            }
        }

        for _ in news.len()..olds.len() {
            out.push(change(ClipChangeKind::EffectRemoved {
                effect: name.to_string(),
            }));
        }
        for _ in olds.len()..news.len() {
            out.push(change(ClipChangeKind::EffectAdded {
                effect: name.to_string(),
            }));
        }
    }
}

fn diff_params(old: &Effect, new: &Effect) -> Vec<ParamChange> {
    let mut out = Vec::new();
    let keys: BTreeSet<&String> = old.params.keys().chain(new.params.keys()).collect();

    for key in keys {
        let a = old.params.get(key);
        let b = new.params.get(key);
        if a == b {
            continue;
        }
        out.push(ParamChange {
            name: key.clone(),
            from: a.cloned().unwrap_or_default(),
            to: b.cloned().unwrap_or_default(),
        });
    }
    out
}

fn diff_markers(before: &Project, after: &Project) -> Vec<MarkerChange> {
    let old: Vec<&Marker> = before.sequences.iter().flat_map(|s| &s.markers).collect();
    let new: Vec<&Marker> = after.sequences.iter().flat_map(|s| &s.markers).collect();
    let mut out = Vec::new();
    let mut used: BTreeSet<usize> = BTreeSet::new();

    for o in &old {
        // Same position is the same marker, retitled or not.
        if let Some((j, n)) = new
            .iter()
            .enumerate()
            .find(|(j, n)| !used.contains(j) && n.position == o.position)
        {
            used.insert(j);
            if n.comment != o.comment {
                out.push(MarkerChange::Retitled {
                    position: o.position,
                    from: o.comment.clone(),
                    to: n.comment.clone(),
                });
            }
            continue;
        }
        // Otherwise the same text elsewhere means it moved.
        if let Some((j, n)) = new
            .iter()
            .enumerate()
            .find(|(j, n)| !used.contains(j) && n.comment == o.comment)
        {
            used.insert(j);
            out.push(MarkerChange::Moved {
                from: o.position,
                to: n.position,
                comment: o.comment.clone(),
            });
            continue;
        }
        out.push(MarkerChange::Removed {
            position: o.position,
            comment: o.comment.clone(),
        });
    }

    for (j, n) in new.iter().enumerate() {
        if !used.contains(&j) {
            out.push(MarkerChange::Added {
                position: n.position,
                comment: n.comment.clone(),
            });
        }
    }
    out
}

fn diff_guides(before: &Project, after: &Project) -> Vec<GuideChange> {
    let mut out = Vec::new();
    let old: BTreeMap<Frames, &Guide> = before.guides.iter().map(|g| (g.position, g)).collect();
    let new: BTreeMap<Frames, &Guide> = after.guides.iter().map(|g| (g.position, g)).collect();

    for (pos, o) in &old {
        match new.get(pos) {
            None => out.push(GuideChange::Removed {
                position: *pos,
                comment: o.comment.clone(),
            }),
            Some(n) if n.comment != o.comment => out.push(GuideChange::Retitled {
                position: *pos,
                from: o.comment.clone(),
                to: n.comment.clone(),
            }),
            Some(_) => {}
        }
    }
    for (pos, n) in &new {
        if !old.contains_key(pos) {
            out.push(GuideChange::Added {
                position: *pos,
                comment: n.comment.clone(),
            });
        }
    }
    out
}
