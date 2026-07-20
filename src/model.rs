//! The structured form of a Kdenlive project, as produced by the parser and
//! consumed by the differ.
//!
//! Everything here is deliberately Kdenlive-shaped. Track kinds are video/audio
//! because that is what Kdenlive has, and effect parameters keep MLT's own
//! keyframe encoding rather than some normalized form, because a diff that
//! talks about anything else would be hard to trace back to the file.

use std::collections::BTreeMap;

/// Frame count on the project timeline. Kdenlive stores most edit points as
/// timecode strings, which we resolve to frames using the project profile so
/// that comparisons are exact integers rather than float seconds.
pub type Frames = i64;

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub profile: Profile,
    /// Bin clips by their `kdenlive:id`, which is the id timeline entries
    /// reference and the only id stable across saves.
    pub bin_clips: BTreeMap<String, BinClip>,
    pub sequences: Vec<Sequence>,
    pub guides: Vec<Guide>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub width: u32,
    pub height: u32,
    pub frame_rate_num: u32,
    pub frame_rate_den: u32,
    pub description: Option<String>,
}

impl Profile {
    pub fn fps(&self) -> f64 {
        if self.frame_rate_den == 0 {
            return 0.0;
        }
        f64::from(self.frame_rate_num) / f64::from(self.frame_rate_den)
    }
}

/// A clip in the project bin. `resource` is a path for media clips, or inline
/// data for the clip types Kdenlive stores in the file itself (titles, colors).
#[derive(Debug, Clone, PartialEq)]
pub struct BinClip {
    pub id: String,
    pub name: Option<String>,
    pub resource: Option<String>,
    pub service: Option<String>,
}

impl BinClip {
    /// What to call this clip in diff output. Prefers the user-visible bin
    /// name, falls back to the file name, then the raw id.
    pub fn label(&self) -> String {
        if let Some(name) = self.name.as_ref().filter(|n| !n.is_empty()) {
            return name.clone();
        }
        if let Some(res) = &self.resource {
            if let Some(base) = res.rsplit('/').next().filter(|b| !b.is_empty()) {
                return base.to_string();
            }
        }
        format!("clip {}", self.id)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sequence {
    pub uuid: String,
    pub name: Option<String>,
    pub tracks: Vec<Track>,
    pub markers: Vec<Marker>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// Display name as an editor knows it: V1, V2, A1, A2.
    pub name: String,
    pub kind: TrackKind,
    pub locked: bool,
    pub clips: Vec<TimelineClip>,
}

/// One clip instance on the timeline. Kdenlive splits each track into two MLT
/// playlists so that same-track mixes have somewhere to live, so a single track
/// here is the merge of both.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineClip {
    /// `kdenlive:id` of the bin clip this instance plays.
    pub bin_id: String,
    pub position: Frames,
    /// In and out points within the source, inclusive of `out` as MLT counts it.
    pub source_in: Frames,
    pub source_out: Frames,
    pub effects: Vec<Effect>,
}

impl TimelineClip {
    pub fn duration(&self) -> Frames {
        self.source_out - self.source_in + 1
    }

    pub fn end(&self) -> Frames {
        self.position + self.duration()
    }
}

/// An MLT filter. `name` is the Kdenlive-facing effect id when present
/// (`kdenlive_id`), otherwise the raw MLT service name.
#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    pub name: String,
    pub service: String,
    pub disabled: bool,
    /// Parameters excluding MLT bookkeeping and Kdenlive UI state, so that a
    /// collapsed panel or a reassigned internal id never reads as an edit.
    pub params: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    pub position: Frames,
    pub comment: String,
    pub category: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Guide {
    pub position: Frames,
    pub comment: String,
}
