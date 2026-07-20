//! Parses a `.kdenlive` file into [`Project`].
//!
//! Kdenlive writes MLT XML in the gen-5 layout: bin clips are `<chain>` or
//! `<producer>` elements, each timeline track is a `<tractor>` wrapping two
//! `<playlist>` elements, and the sequence itself is a `<tractor>` carrying a
//! `kdenlive:uuid`. See dev-docs/fileformat.md in the Kdenlive tree.
//!
//! The parser refuses to guess. If a document is a generation we do not handle,
//! or a reference does not resolve, it returns an error instead of a project
//! with holes in it, because a diff computed from a partial parse would quietly
//! misreport what the editor changed.

mod json_lite;
mod timecode;

use std::collections::BTreeMap;

use roxmltree::{Document, Node};

use crate::model::{
    BinClip, Effect, Frames, Guide, Marker, Profile, Project, Sequence, TimelineClip, Track,
    TrackKind,
};

pub use timecode::{format_duration, parse_timecode};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("not valid XML: {0}")]
    Xml(#[from] roxmltree::Error),

    #[error("not a Kdenlive project file: root element is <{0}>, expected <mlt>")]
    NotKdenlive(String),

    #[error(
        "unsupported project format (document version {0}). \
         cutback handles version 1.1, written by Kdenlive 23.04 and later. \
         Open the project in Kdenlive once and save it to upgrade the format"
    )]
    UnsupportedVersion(String),

    #[error("project has no <profile>, cannot resolve timecodes to frames")]
    MissingProfile,

    #[error("profile is missing {0}")]
    BadProfile(&'static str),

    #[error("{context}: expected an integer, found {value:?}")]
    NotAnInteger { context: String, value: String },

    #[error("timeline entry references producer {0:?}, which is not defined in this file")]
    DanglingProducer(String),

    #[error("track tractor {0:?} references playlist {1:?}, which is not defined in this file")]
    DanglingPlaylist(String, String),

    #[error("bad timecode {value:?} in {context}: {reason}")]
    BadTimecode {
        context: String,
        value: String,
        reason: &'static str,
    },

    #[error("project contains no timeline sequence")]
    NoSequence,

    #[error("malformed {kind} marker {value:?}: {reason}")]
    BadMarker {
        kind: &'static str,
        value: String,
        reason: &'static str,
    },
}

type Result<T> = std::result::Result<T, ParseError>;

/// Version 1.1 is gen-5, introduced in Kdenlive 23.04 along with multiple
/// sequences. The parser locates the timeline by looking for the tractor
/// carrying a `kdenlive:uuid`, which only exists from that version on, so
/// older documents are refused rather than parsed into an empty timeline.
/// 1.1 is also Kdenlive's current DOCUMENTVERSION, and anything newer is
/// refused too: a format bump is exactly where a silently wrong diff comes from.
const MIN_VERSION: f64 = 1.1;
const MAX_VERSION: f64 = 1.1;

/// MLT and Kdenlive bookkeeping that changes without the project changing.
/// Dropping these keeps a save with no real edits from producing a diff.
const IGNORED_EFFECT_PARAMS: &[&str] = &[
    "kdenlive:collapsed",
    "kdenlive_id",
    "mlt_service",
    "shotcut:filter",
    "_loaded",
    "internal_added",
];

pub fn parse_file(path: &std::path::Path) -> anyhow::Result<Project> {
    let text = std::fs::read_to_string(path)?;
    parse_str(&text).map_err(|e| anyhow::anyhow!("{}: {}", path.display(), e))
}

pub fn parse_str(text: &str) -> Result<Project> {
    let doc = Document::parse(text)?;
    let root = doc.root_element();
    if root.tag_name().name() != "mlt" {
        return Err(ParseError::NotKdenlive(root.tag_name().name().to_string()));
    }

    let profile = parse_profile(root)?;
    let fps = profile.fps();

    // Index every element that can be referenced by id before walking anything,
    // since MLT references point both forward and backward in the document.
    let mut producers: BTreeMap<&str, Node> = BTreeMap::new();
    let mut playlists: BTreeMap<&str, Node> = BTreeMap::new();
    let mut tractors: Vec<Node> = Vec::new();

    for node in root.children().filter(Node::is_element) {
        let id = node.attribute("id");
        match node.tag_name().name() {
            // Both spell "a source of frames". Modern Kdenlive emits <chain>
            // for avformat media and <producer> for the rest.
            "chain" | "producer" => {
                if let Some(id) = id {
                    producers.insert(id, node);
                }
            }
            "playlist" => {
                if let Some(id) = id {
                    playlists.insert(id, node);
                }
            }
            "tractor" => tractors.push(node),
            _ => {}
        }
    }

    let main_bin = playlists.get("main_bin").copied();
    if let Some(bin) = main_bin {
        check_version(bin)?;
    }

    let bin_clips = parse_bin_clips(&producers, main_bin)?;
    let guides = main_bin.map(parse_guides).transpose()?.unwrap_or_default();

    let sequences = parse_sequences(&tractors, &playlists, &producers, fps)?;
    if sequences.is_empty() {
        return Err(ParseError::NoSequence);
    }

    Ok(Project {
        profile,
        bin_clips,
        sequences,
        guides,
    })
}

fn check_version(main_bin: Node) -> Result<()> {
    let Some(version) = property(main_bin, "kdenlive:docproperties.version") else {
        return Ok(());
    };
    let version = version.trim();
    // Kdenlive itself stores and compares this as a double, so "1.04" is an
    // older 1.x file and not a typo for 1.4. Parse rather than string match.
    let parsed: f64 = version
        .parse()
        .map_err(|_| ParseError::UnsupportedVersion(version.to_string()))?;
    if (MIN_VERSION..=MAX_VERSION).contains(&parsed) {
        Ok(())
    } else {
        Err(ParseError::UnsupportedVersion(version.to_string()))
    }
}

fn parse_profile(root: Node) -> Result<Profile> {
    let node = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "profile")
        .ok_or(ParseError::MissingProfile)?;

    let attr = |name: &'static str| -> Result<u32> {
        node.attribute(name)
            .ok_or(ParseError::BadProfile(name))?
            .parse::<u32>()
            .map_err(|_| ParseError::BadProfile(name))
    };

    Ok(Profile {
        width: attr("width")?,
        height: attr("height")?,
        frame_rate_num: attr("frame_rate_num")?,
        frame_rate_den: attr("frame_rate_den")?,
        description: node.attribute("description").map(str::to_string),
    })
}

fn parse_bin_clips(
    producers: &BTreeMap<&str, Node>,
    main_bin: Option<Node>,
) -> Result<BTreeMap<String, BinClip>> {
    let mut clips = BTreeMap::new();

    // The bin lists what the user actually sees. Producers not listed there are
    // internal (sequence wrappers, black track) and would be noise in a diff.
    let listed: Option<Vec<&str>> = main_bin.map(|bin| {
        bin.children()
            .filter(|n| n.is_element() && n.tag_name().name() == "entry")
            .filter_map(|n| n.attribute("producer"))
            .collect()
    });

    for (elem_id, node) in producers {
        if let Some(listed) = &listed {
            if !listed.contains(elem_id) {
                continue;
            }
        }
        // kdenlive:id is what timeline entries reference. Producers without one
        // are MLT internals, not bin clips.
        let Some(kid) = property(*node, "kdenlive:id") else {
            continue;
        };
        clips.insert(
            kid.clone(),
            BinClip {
                id: kid,
                name: property(*node, "kdenlive:clipname"),
                resource: property(*node, "resource"),
                service: property(*node, "mlt_service"),
            },
        );
    }

    Ok(clips)
}

fn parse_sequences(
    tractors: &[Node],
    playlists: &BTreeMap<&str, Node>,
    producers: &BTreeMap<&str, Node>,
    fps: f64,
) -> Result<Vec<Sequence>> {
    let mut sequences = Vec::new();

    for tractor in tractors {
        // A sequence tractor is the one carrying a uuid. The final tractor with
        // kdenlive:projectTractor is only a playback wrapper, and the per-track
        // tractors carry neither.
        let Some(uuid) = property(*tractor, "kdenlive:uuid") else {
            continue;
        };

        let mut tracks = Vec::new();
        let mut video_index = 0usize;
        let mut audio_index = 0usize;

        for track_ref in tractor
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "track")
        {
            let Some(producer_id) = track_ref.attribute("producer") else {
                continue;
            };
            // The black_track sits in the track list but is not a user track.
            let Some(track_tractor) = tractors.iter().find(|t| t.attribute("id") == Some(producer_id))
            else {
                continue;
            };

            let kind = if property(*track_tractor, "kdenlive:audio_track").as_deref() == Some("1") {
                TrackKind::Audio
            } else {
                TrackKind::Video
            };

            let clips = parse_track_clips(*track_tractor, playlists, producers, fps)?;

            let name = match kind {
                TrackKind::Video => {
                    video_index += 1;
                    format!("V{video_index}")
                }
                TrackKind::Audio => {
                    audio_index += 1;
                    format!("A{audio_index}")
                }
            };

            tracks.push(Track {
                name,
                kind,
                locked: property(*track_tractor, "kdenlive:locked_track").as_deref() == Some("1"),
                clips,
            });
        }

        // Kdenlive lists tracks bottom-up, so the first video track in the file
        // is V1. Video tracks are numbered upward from the bottom, which the
        // counters above already produce, but audio tracks read downward, so
        // reverse the display order to match what the editor sees.
        reorder_tracks_for_display(&mut tracks);

        sequences.push(Sequence {
            uuid,
            name: property(*tractor, "kdenlive:clipname"),
            markers: parse_markers(*tractor)?,
            tracks,
        });
    }

    Ok(sequences)
}

/// Kdenlive's timeline shows video tracks above audio tracks, with V1 nearest
/// the middle and audio counting downward. The file order is bottom-up, so
/// present video tracks in reverse and audio in file order.
fn reorder_tracks_for_display(tracks: &mut Vec<Track>) {
    let (mut video, audio): (Vec<_>, Vec<_>) = std::mem::take(tracks)
        .into_iter()
        .partition(|t| t.kind == TrackKind::Video);
    video.reverse();
    tracks.extend(video);
    tracks.extend(audio);
}

fn parse_track_clips(
    track_tractor: Node,
    playlists: &BTreeMap<&str, Node>,
    producers: &BTreeMap<&str, Node>,
    fps: f64,
) -> Result<Vec<TimelineClip>> {
    let tractor_id = track_tractor.attribute("id").unwrap_or("?");
    let mut clips = Vec::new();

    // A Kdenlive track is two MLT playlists so that same-track mixes have a
    // second lane to live on. Both hold real clips, so walk both and sort by
    // position afterwards.
    for track_ref in track_tractor
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "track")
    {
        let Some(playlist_id) = track_ref.attribute("producer") else {
            continue;
        };
        let playlist = playlists.get(playlist_id).copied().ok_or_else(|| {
            ParseError::DanglingPlaylist(tractor_id.to_string(), playlist_id.to_string())
        })?;

        // Position is implicit: entries and blanks lay end to end, so the
        // running total of what came before is where the next clip starts.
        let mut playhead: Frames = 0;

        for child in playlist.children().filter(Node::is_element) {
            match child.tag_name().name() {
                "blank" => {
                    let length = child.attribute("length").ok_or_else(|| ParseError::BadTimecode {
                        context: format!("blank in {playlist_id}"),
                        value: String::new(),
                        reason: "missing length attribute",
                    })?;
                    playhead += parse_timecode(length, fps).map_err(|reason| {
                        ParseError::BadTimecode {
                            context: format!("blank in {playlist_id}"),
                            value: length.to_string(),
                            reason,
                        }
                    })?;
                }
                "entry" => {
                    let clip = parse_entry(child, playlist_id, playhead, producers, fps)?;
                    playhead = clip.end();
                    clips.push(clip);
                }
                _ => {}
            }
        }
    }

    clips.sort_by_key(|c| (c.position, c.bin_id.clone()));
    Ok(clips)
}

fn parse_entry(
    entry: Node,
    playlist_id: &str,
    position: Frames,
    producers: &BTreeMap<&str, Node>,
    fps: f64,
) -> Result<TimelineClip> {
    let producer_id = entry
        .attribute("producer")
        .ok_or_else(|| ParseError::DanglingProducer(format!("<entry> in {playlist_id}")))?;

    // Resolve through the producer so an entry always maps to a bin clip, even
    // when the entry itself carries no kdenlive:id.
    let bin_id = property(entry, "kdenlive:id")
        .or_else(|| {
            producers
                .get(producer_id)
                .and_then(|p| property(*p, "kdenlive:id"))
        })
        .ok_or_else(|| ParseError::DanglingProducer(producer_id.to_string()))?;

    let context = || format!("<entry producer={producer_id:?}> in {playlist_id}");
    let read = |name: &str| -> Result<Frames> {
        let raw = entry.attribute(name).ok_or_else(|| ParseError::BadTimecode {
            context: context(),
            value: String::new(),
            reason: "missing in/out attribute",
        })?;
        parse_timecode(raw, fps).map_err(|reason| ParseError::BadTimecode {
            context: context(),
            value: raw.to_string(),
            reason,
        })
    };

    Ok(TimelineClip {
        bin_id,
        position,
        source_in: read("in")?,
        source_out: read("out")?,
        effects: parse_effects(entry),
    })
}

fn parse_effects(parent: Node) -> Vec<Effect> {
    parent
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "filter")
        .filter_map(|filter| {
            let service = property(filter, "mlt_service")?;
            // Filters Kdenlive adds on its own behalf (audio mixing, subtitle
            // rendering) are not user edits and should never appear in a diff.
            if property(filter, "internal_added").is_some() {
                return None;
            }
            let name = property(filter, "kdenlive_id").unwrap_or_else(|| service.clone());
            let mut params = BTreeMap::new();
            for prop in filter.children().filter(|n| n.is_element() && n.tag_name().name() == "property") {
                let Some(key) = prop.attribute("name") else {
                    continue;
                };
                if IGNORED_EFFECT_PARAMS.contains(&key) {
                    continue;
                }
                params.insert(key.to_string(), prop.text().unwrap_or_default().to_string());
            }
            let disabled = params.remove("disable").as_deref() == Some("1");
            Some(Effect {
                name,
                service,
                disabled,
                params,
            })
        })
        .collect()
}

/// Markers live on the sequence tractor as a `kdenlive:markers` JSON array.
/// Positions there are already frame numbers, not seconds, so they need no
/// conversion through the profile.
fn parse_markers(tractor: Node) -> Result<Vec<Marker>> {
    let Some(raw) = property(tractor, "kdenlive:markers") else {
        return Ok(Vec::new());
    };
    let objects = json_lite::parse_object_array(&raw).ok_or_else(|| ParseError::BadMarker {
        kind: "timeline",
        value: truncate(&raw),
        reason: "expected a JSON array of marker objects",
    })?;

    let mut markers = Vec::with_capacity(objects.len());
    for obj in objects {
        let pos = obj.num("pos").ok_or_else(|| ParseError::BadMarker {
            kind: "timeline",
            value: truncate(&raw),
            reason: "marker has no numeric pos",
        })?;
        markers.push(Marker {
            position: pos.round() as Frames,
            comment: obj.str("comment").unwrap_or_default().to_string(),
            category: obj.num("type").unwrap_or(0.0) as i32,
        });
    }
    markers.sort_by_key(|m| m.position);
    Ok(markers)
}

/// Guides are stored on the bin in the same JSON shape as markers.
fn parse_guides(main_bin: Node) -> Result<Vec<Guide>> {
    let Some(raw) = property(main_bin, "kdenlive:docproperties.guides") else {
        return Ok(Vec::new());
    };
    let objects = json_lite::parse_object_array(&raw).ok_or_else(|| ParseError::BadMarker {
        kind: "guide",
        value: truncate(&raw),
        reason: "expected a JSON array of guide objects",
    })?;

    let mut guides = Vec::with_capacity(objects.len());
    for obj in objects {
        let pos = obj.num("pos").ok_or_else(|| ParseError::BadMarker {
            kind: "guide",
            value: truncate(&raw),
            reason: "guide has no numeric pos",
        })?;
        guides.push(Guide {
            position: pos.round() as Frames,
            comment: obj.str("comment").unwrap_or_default().to_string(),
        });
    }
    guides.sort_by_key(|g| g.position);
    Ok(guides)
}

/// Keeps a malformed property from dumping a whole JSON blob into an error.
fn truncate(raw: &str) -> String {
    let flat = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 80 {
        format!("{}...", flat.chars().take(80).collect::<String>())
    } else {
        flat
    }
}

/// Reads an MLT `<property name="...">value</property>` child.
fn property(node: Node, name: &str) -> Option<String> {
    node.children()
        .filter(|n| n.is_element() && n.tag_name().name() == "property")
        .find(|n| n.attribute("name") == Some(name))
        .map(|n| n.text().unwrap_or_default().to_string())
}
