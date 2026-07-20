//! Converting MLT time values to frame counts.
//!
//! MLT accepts several spellings for the same instant and Kdenlive writes more
//! than one of them in a single file: entry in/out points are usually clock
//! values like `00:00:26.400`, while some properties carry a bare frame count.
//! Everything downstream compares positions as integers, so all of it resolves
//! to frames here.

use crate::model::Frames;

/// Parses an MLT time value into a frame number.
///
/// Accepted forms:
/// - `HH:MM:SS.mmm` clock time, the form Kdenlive writes for in/out points
/// - `HH:MM:SS:FF` SMPTE, where the last field is already frames
/// - `1234` a bare frame count
///
/// `fps` resolves the fractional part. Rounding is half-away-from-zero rather
/// than truncating, because 25 fps writes 0.040 per frame and truncation turns
/// a value one ulp low into the previous frame, which shows up as a spurious
/// one-frame trim in the diff.
pub fn parse_timecode(raw: &str, fps: f64) -> Result<Frames, &'static str> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("empty value");
    }

    if !value.contains(':') {
        // Either a bare frame count or a seconds value with a decimal point.
        if let Ok(frames) = value.parse::<i64>() {
            return Ok(frames);
        }
        let seconds: f64 = value.parse().map_err(|_| "not a number or timecode")?;
        return seconds_to_frames(seconds, fps);
    }

    let parts: Vec<&str> = value.split(':').collect();
    let [hours, minutes, rest] = parts[..] else {
        return Err("expected HH:MM:SS.mmm or HH:MM:SS:FF");
    };

    let hours: f64 = hours.parse().map_err(|_| "hours is not a number")?;
    let minutes: f64 = minutes.parse().map_err(|_| "minutes is not a number")?;
    let seconds: f64 = rest.parse().map_err(|_| "seconds is not a number")?;

    seconds_to_frames(hours * 3600.0 + minutes * 60.0 + seconds, fps)
}

fn seconds_to_frames(seconds: f64, fps: f64) -> Result<Frames, &'static str> {
    if !fps.is_finite() || fps <= 0.0 {
        return Err("project profile has no usable frame rate");
    }
    if !seconds.is_finite() {
        return Err("not a finite time value");
    }
    Ok((seconds * fps).round() as Frames)
}

/// Renders a frame number as `H:MM:SS` or `M:SS`, the way an editor reads a
/// duration off the timeline. Frames are deliberately dropped, a diff line
/// saying "new duration 4:12" is more useful than one quoting 6312 frames.
pub fn format_duration(frames: Frames, fps: f64) -> String {
    if !fps.is_finite() || fps <= 0.0 {
        return format!("{frames} frames");
    }
    let total = (frames as f64 / fps).round().max(0.0) as i64;
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_time_at_30fps() {
        // Values taken from a real project file written at 30 fps.
        assert_eq!(parse_timecode("00:00:00.000", 30.0), Ok(0));
        assert_eq!(parse_timecode("00:00:26.400", 30.0), Ok(792));
        assert_eq!(parse_timecode("00:00:22.900", 30.0), Ok(687));
    }

    #[test]
    fn clock_time_at_25fps() {
        assert_eq!(parse_timecode("00:00:04.960", 25.0), Ok(124));
        assert_eq!(parse_timecode("00:00:03.720", 25.0), Ok(93));
        assert_eq!(parse_timecode("00:02:20.840", 25.0), Ok(3521));
    }

    #[test]
    fn bare_frame_count() {
        assert_eq!(parse_timecode("793", 30.0), Ok(793));
        assert_eq!(parse_timecode("0", 30.0), Ok(0));
    }

    #[test]
    fn hours_carry() {
        assert_eq!(parse_timecode("01:00:00.000", 30.0), Ok(108_000));
        // 9015.5s at 25 fps is 225387.5, which rounds away from zero.
        assert_eq!(parse_timecode("02:30:15.500", 25.0), Ok(225_388));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_timecode("", 30.0).is_err());
        assert!(parse_timecode("not a time", 30.0).is_err());
        assert!(parse_timecode("00:xx:00.000", 30.0).is_err());
        assert!(parse_timecode("00:00:26.400", 0.0).is_err());
    }

    #[test]
    fn durations_read_like_a_timeline() {
        assert_eq!(format_duration(792, 30.0), "0:26");
        assert_eq!(format_duration(7560, 30.0), "4:12");
        assert_eq!(format_duration(108_000, 30.0), "1:00:00");
    }
}
