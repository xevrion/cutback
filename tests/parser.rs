//! Parser tests against real project files.
//!
//! Every file in tests/data was written by Kdenlive itself, not hand authored.
//! The timelapse-* files are three consecutive saves of one project, which is
//! also what the differ tests build on.

use std::path::PathBuf;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
}

fn parse(name: &str) -> cutback::model::Project {
    cutback::xml_parser::parse_file(&data(name))
        .unwrap_or_else(|e| panic!("{name} should parse: {e}"))
}

#[test]
fn reads_the_project_profile() {
    let p = parse("timelapse-a.kdenlive");
    assert_eq!((p.profile.width, p.profile.height), (1920, 1080));
    assert_eq!(p.profile.fps(), 30.0);
    assert_eq!(p.profile.description.as_deref(), Some("HD 1080p 30 fps"));
}

/// Modern Kdenlive writes avformat media as <chain>, not <producer>. A parser
/// that only looked for <producer> would find an empty bin here.
#[test]
fn finds_bin_clips_written_as_chains() {
    let p = parse("timelapse-a.kdenlive");
    let media: Vec<_> = p
        .bin_clips
        .values()
        .filter(|c| c.service.as_deref() == Some("avformat-novalidate"))
        .collect();
    assert!(
        media.len() >= 5,
        "expected the timelapse footage in the bin, found {}",
        media.len()
    );
    assert!(media.iter().any(|c| c.label() == "VID_20251212_092005.mp4"));
}

#[test]
fn title_clips_keep_their_bin_name() {
    let p = parse("timelapse-b.kdenlive");
    let title = p
        .bin_clips
        .values()
        .find(|c| c.service.as_deref() == Some("kdenlivetitle"))
        .expect("this save added a title clip");
    // Titles have no resource path, so the bin name is the only usable label.
    assert!(!title.label().is_empty());
    assert!(!title.label().starts_with("clip "));
}

#[test]
fn tracks_are_named_the_way_the_editor_sees_them() {
    let p = parse("timelapse-a.kdenlive");
    let seq = &p.sequences[0];
    let names: Vec<&str> = seq.tracks.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["V2", "V1", "A1", "A2"]);

    let kinds: Vec<_> = seq.tracks.iter().map(|t| t.kind).collect();
    use cutback::model::TrackKind::{Audio, Video};
    assert_eq!(kinds, vec![Video, Video, Audio, Audio]);
}

/// Clip positions are implicit in MLT: each entry starts where the previous
/// entry or blank ended. Getting this wrong shifts every clip after a gap.
#[test]
fn clips_lay_end_to_end_on_the_track() {
    let p = parse("timelapse-a.kdenlive");
    let v1 = p.sequences[0]
        .tracks
        .iter()
        .find(|t| t.name == "V1")
        .expect("V1 exists");

    assert_eq!(v1.clips.len(), 5);
    assert_eq!(v1.clips[0].position, 0);
    for pair in v1.clips.windows(2) {
        assert_eq!(
            pair[1].position,
            pair[0].end(),
            "clip at {} should start where the previous one ended",
            pair[1].position
        );
    }
}

#[test]
fn clip_durations_come_from_in_and_out_points() {
    let p = parse("timelapse-a.kdenlive");
    let v1 = p.sequences[0].tracks.iter().find(|t| t.name == "V1").unwrap();
    let first = &v1.clips[0];
    // in=0 out=228 is 229 frames, MLT counts the out point as included.
    assert_eq!((first.source_in, first.source_out), (0, 228));
    assert_eq!(first.duration(), 229);
}

/// Markers are a kdenlive:markers JSON array with positions already in frames.
/// The format doc still describes the older position:comment spelling.
#[test]
fn reads_json_markers() {
    let p = parse("markers.kdenlive");
    let markers: Vec<_> = p.sequences.iter().flat_map(|s| &s.markers).collect();
    assert!(!markers.is_empty(), "this project has a timeline marker");
    assert!(markers.iter().any(|m| m.comment == "Gap"));
    assert!(markers.iter().any(|m| m.position == 2574));
}

#[test]
fn every_sequence_has_a_uuid() {
    for name in ["timelapse-a.kdenlive", "timelapse-b.kdenlive", "markers.kdenlive"] {
        let p = parse(name);
        assert!(!p.sequences.is_empty(), "{name} has a timeline");
        for seq in &p.sequences {
            assert!(seq.uuid.starts_with('{'), "{name}: sequence uuid looks wrong");
        }
    }
}

/// Kdenlive's own audio mixing and subtitle filters are not user edits, and
/// reporting them would make every diff noisy.
#[test]
fn internal_filters_are_not_treated_as_effects() {
    let p = parse("timelapse-a.kdenlive");
    for seq in &p.sequences {
        for track in &seq.tracks {
            for clip in &track.clips {
                for effect in &clip.effects {
                    assert_ne!(effect.service, "panner");
                    assert_ne!(effect.service, "audiolevel");
                }
            }
        }
    }
}

#[test]
fn refuses_files_that_are_not_kdenlive_projects() {
    let err = cutback::xml_parser::parse_str("<html><body/></html>").unwrap_err();
    assert!(err.to_string().contains("not a Kdenlive project"));
}

#[test]
fn refuses_malformed_xml() {
    assert!(cutback::xml_parser::parse_str("<mlt><unclosed>").is_err());
}

/// A future format is exactly where a silently wrong diff would come from, so
/// the parser refuses rather than guessing.
#[test]
fn refuses_a_newer_document_version() {
    let doc = r#"<?xml version='1.0' encoding='utf-8'?>
<mlt producer="main_bin">
 <profile width="1920" height="1080" frame_rate_num="30" frame_rate_den="1"/>
 <playlist id="main_bin">
  <property name="kdenlive:docproperties.version">2.0</property>
 </playlist>
</mlt>"#;
    let err = cutback::xml_parser::parse_str(doc).unwrap_err();
    assert!(err.to_string().contains("unsupported project format"), "{err}");
}
