use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptionCue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// YouTube json3 captions preserve timestamps; roll-up duplicates are omitted
/// only when text and start time are identical.
pub fn parse_json3(value: &Value) -> Result<Vec<CaptionCue>> {
    let mut cues: Vec<CaptionCue> = vec![];
    for event in value["events"]
        .as_array()
        .context("Caption file has no events")?
    {
        let Some(start) = event["tStartMs"].as_u64() else {
            continue;
        };
        let Some(parts) = event["segs"].as_array() else {
            continue;
        };
        let text = parts
            .iter()
            .filter_map(|part| part["utf8"].as_str())
            .collect::<String>();
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        let end = start.saturating_add(event["dDurationMs"].as_u64().unwrap_or(3000));
        if end <= start
            || cues
                .last()
                .is_some_and(|cue| cue.start_ms == start && cue.text == text)
        {
            continue;
        }
        cues.push(CaptionCue {
            start_ms: start,
            end_ms: end,
            text,
        });
    }
    ensure!(!cues.is_empty(), "Caption file contains no timed text");
    cues.sort_by_key(|cue| cue.start_ms);
    Ok(cues)
}

pub fn excerpt(path: &str, start_ms: u64, end_ms: u64) -> Result<Vec<CaptionCue>> {
    let cues: Vec<CaptionCue> = serde_json::from_slice(&std::fs::read(path)?)?;
    Ok(cues
        .into_iter()
        .filter(|cue| cue.end_ms > start_ms && cue.start_ms < end_ms)
        .take(400)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn joins_segments_and_keeps_source_timing_without_control_events() {
        let cues = parse_json3(&serde_json::json!({"events":[
            {"tStartMs":0,"wWinId":1},
            {"tStartMs":1200,"dDurationMs":900,"segs":[{"utf8":"Get "},{"utf8":"the key\n"}]},
            {"tStartMs":1200,"dDurationMs":900,"segs":[{"utf8":"Get the key"}]},
            {"tStartMs":4000,"dDurationMs":2000,"segs":[{"utf8":"Go left"}]}
        ]}))
        .unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "Get the key");
        assert_eq!((cues[0].start_ms, cues[0].end_ms), (1200, 2100));
    }
}
