//! Differ tests.
//!
//! The timelapse-* files are consecutive saves of a real project. Cases they
//! do not cover (trims, moves, effect edits) are made by editing a real file's
//! XML in place, so the input is still what Kdenlive wrote apart from the one
//! value under test.

use std::path::PathBuf;

use cutback::differ::{diff, ClipChangeKind};
use cutback::model::Project;
use cutback::render::{render, summarize};

fn read(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn parse(text: &str) -> Project {
    cutback::xml_parser::parse_str(text).expect("sample should parse")
}

fn lines(before: &Project, after: &Project) -> Vec<String> {
    render(&diff(before, after), after.profile.fps())
}

/// Kdenlive rewrites the file on every save, and element order is not stable
/// even when nothing was edited. These two saves differ only by a producer
/// moving within the document, so the diff has to be empty. If this fails,
/// the daemon would commit noise on every save.
#[test]
fn reordered_xml_is_not_a_change() {
    let a = parse(&read("timelapse-a.kdenlive"));
    let b = parse(&read("timelapse-b.kdenlive"));
    let d = diff(&a, &b);
    assert!(
        d.is_empty(),
        "expected no changes, got {:?}",
        render(&d, 30.0)
    );
    assert_eq!(summarize(&d, 30.0), "saved with no detected changes");
}

#[test]
fn a_file_against_itself_has_no_changes() {
    for name in [
        "timelapse-a.kdenlive",
        "timelapse-c.kdenlive",
        "markers.kdenlive",
    ] {
        let p = parse(&read(name));
        assert!(
            diff(&p, &p).is_empty(),
            "{name} should not differ from itself"
        );
    }
}

#[test]
fn detects_clips_and_bin_items_added_between_real_saves() {
    let b = parse(&read("timelapse-b.kdenlive"));
    let c = parse(&read("timelapse-c.kdenlive"));
    let out = lines(&b, &c);

    assert!(
        out.iter().any(|l| l.contains("to the project bin")),
        "expected a bin addition, got {out:?}"
    );
    assert!(
        out.iter()
            .any(|l| l.starts_with("added") && l.contains(" to V2 at ")),
        "expected clips added to V2, got {out:?}"
    );
}

#[test]
fn removals_mirror_additions() {
    let b = parse(&read("timelapse-b.kdenlive"));
    let c = parse(&read("timelapse-c.kdenlive"));

    let forward = lines(&b, &c);
    let back = lines(&c, &b);
    assert_eq!(forward.len(), back.len());
    assert!(back.iter().all(|l| l.starts_with("removed")), "{back:?}");
}

/// Moving a clip must read as a move, not as a delete plus an add. The clip
/// has no stable id across saves, so this exercises the position matching.
#[test]
fn moving_a_clip_reads_as_a_move() {
    let original = read("timelapse-a.kdenlive");
    // V2 holds one title clip after a blank. Lengthening the blank slides the
    // clip later on the track, which is what dragging it in Kdenlive does.
    let moved = original.replacen(
        r#"<blank length="00:00:19.700"/>"#,
        r#"<blank length="00:00:21.700"/>"#,
        1,
    );
    assert_ne!(original, moved, "the blank we key on should exist");

    let d = diff(&parse(&original), &parse(&moved));
    let moves: Vec<_> = d
        .clip_changes
        .iter()
        .filter(|c| matches!(c.kind, ClipChangeKind::Moved { .. }))
        .collect();

    assert_eq!(
        moves.len(),
        1,
        "expected one move, got {:?}",
        d.clip_changes
    );
    assert_eq!(moves[0].track, "V2");
    assert!(
        !d.clip_changes.iter().any(|c| matches!(
            c.kind,
            ClipChangeKind::Added { .. } | ClipChangeKind::Removed { .. }
        )),
        "a move must not also report an add or a remove"
    );
}

/// Changing an out point is a trim, and the rendered line should say so in
/// durations an editor recognizes rather than frame counts.
#[test]
fn trimming_a_clip_reports_old_and_new_duration() {
    let original = read("timelapse-a.kdenlive");
    let trimmed = original.replacen(
        r#"<entry in="00:00:00.000" out="00:00:07.600" producer="producer1">"#,
        r#"<entry in="00:00:00.000" out="00:00:05.600" producer="producer1">"#,
        1,
    );
    assert_ne!(original, trimmed, "the entry we key on should exist");

    let before = parse(&original);
    let after = parse(&trimmed);
    let trims: Vec<_> = diff(&before, &after)
        .clip_changes
        .into_iter()
        .filter(|c| matches!(c.kind, ClipChangeKind::Trimmed { .. }))
        .collect();

    assert_eq!(trims.len(), 1, "expected exactly one trim, got {trims:?}");

    let line = &lines(&before, &after)[0];
    assert!(line.starts_with("trimmed "), "{line}");
    assert!(line.contains("0:08 to 0:06"), "{line}");
}

#[test]
fn extending_a_clip_says_extended_not_trimmed() {
    let original = read("timelapse-a.kdenlive");
    let longer = original.replacen(
        r#"<entry in="00:00:00.000" out="00:00:07.600" producer="producer1">"#,
        r#"<entry in="00:00:00.000" out="00:00:09.600" producer="producer1">"#,
        1,
    );
    let out = lines(&parse(&original), &parse(&longer));
    assert!(out.iter().any(|l| l.starts_with("extended ")), "{out:?}");
}

/// An effect parameter change should name the parameter and both values.
#[test]
fn changing_an_effect_parameter_names_it() {
    let original = read("timelapse-a.kdenlive");
    let louder = original.replacen(
        r#"<property name="gain">0.25</property>"#,
        r#"<property name="gain">0.75</property>"#,
        1,
    );
    assert_ne!(original, louder, "the gain property we key on should exist");

    let out = lines(&parse(&original), &parse(&louder));
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].contains("gain 0.25 to 0.75"), "{}", out[0]);
    assert!(
        out[0].ends_with("A1"),
        "the line should name the track: {}",
        out[0]
    );
    // The effect here is named "gain" and so is its only parameter. Saying it
    // twice reads badly, so the effect name is dropped in that case.
    assert!(!out[0].contains("gain effect"), "{}", out[0]);
}

#[test]
fn marker_changes_are_reported_with_their_text() {
    let original = read("markers.kdenlive");
    let renamed = original.replacen(r#""comment": "Gap""#, r#""comment": "Cut here""#, 1);
    assert_ne!(original, renamed, "the marker we key on should exist");

    let out = lines(&parse(&original), &parse(&renamed));
    assert!(
        out.iter()
            .any(|l| l.contains("Gap") && l.contains("Cut here")),
        "{out:?}"
    );
}

#[test]
fn profile_changes_are_reported() {
    let original = read("timelapse-a.kdenlive");
    let changed = original.replacen(r#"frame_rate_num="30""#, r#"frame_rate_num="60""#, 1);
    let out = lines(&parse(&original), &parse(&changed));
    assert!(
        out.iter()
            .any(|l| l.contains("frame rate") && l.contains("30 fps") && l.contains("60 fps")),
        "{out:?}"
    );
}

/// The commit subject has to stay short no matter how large the edit was.
#[test]
fn summary_stays_one_line() {
    let b = parse(&read("timelapse-b.kdenlive"));
    let c = parse(&read("timelapse-c.kdenlive"));
    let subject = summarize(&diff(&b, &c), 30.0);

    assert!(!subject.contains('\n'), "commit subject must be one line");
    assert!(subject.starts_with(char::is_uppercase), "{subject}");
    assert!(subject.contains("more change"), "{subject}");
}
