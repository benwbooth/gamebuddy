use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Source {
    pub id: String,
    pub title: String,
    pub url: Option<String>,
    pub kind: SourceKind,
    pub duration_ms: Option<u64>,
    pub media_path: Option<String>,
    #[serde(default)]
    pub captions_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Text,
    Youtube,
    Video,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct Anchor {
    pub source_id: String,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub section: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct Step {
    pub id: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub instructions: String,
    #[serde(default)]
    pub prerequisites: Vec<String>,
    #[serde(default)]
    pub next: Vec<String>,
    #[serde(default)]
    pub completion_signals: Vec<String>,
    #[serde(default)]
    pub anchors: Vec<Anchor>,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Guide {
    pub schema_version: u32,
    pub id: String,
    pub revision: u64,
    pub game: String,
    pub title: String,
    pub sources: Vec<Source>,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub coverage_notes: Vec<String>,
    #[serde(default)]
    pub goals: Vec<PlayerGoal>,
    #[serde(default)]
    pub reference_preferences: Vec<ReferencePreference>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct PlayerGoal {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub step_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct ReferencePreference {
    pub source_id: String,
    pub enabled: bool,
    #[serde(default)]
    pub preference: String,
}

impl Guide {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "Unsupported guide schema {}",
            self.schema_version
        );
        ensure!(
            !self.id.trim().is_empty() && !self.game.trim().is_empty(),
            "Guide ID and game are required"
        );
        ensure!(
            self.revision > 0 && self.revision < i64::MAX as u64,
            "Guide revision is out of range"
        );
        ensure!(!self.steps.is_empty(), "The guide has no steps");
        let sources: HashMap<_, _> = self.sources.iter().map(|s| (s.id.as_str(), s)).collect();
        ensure!(sources.len() == self.sources.len(), "Duplicate source IDs");
        let nodes: HashMap<_, _> = self.steps.iter().map(|s| (s.id.as_str(), s)).collect();
        ensure!(nodes.len() == self.steps.len(), "Duplicate step IDs");
        let mut goal_ids = HashSet::new();
        for goal in &self.goals {
            ensure!(
                !goal.id.trim().is_empty() && !goal.description.trim().is_empty(),
                "Goal ID and description are required"
            );
            ensure!(goal_ids.insert(&goal.id), "Duplicate goal ID");
            ensure!(
                goal.step_ids
                    .iter()
                    .all(|id| nodes.contains_key(id.as_str())),
                "Goal references a missing step"
            );
        }
        let mut preference_ids = HashSet::new();
        for preference in &self.reference_preferences {
            ensure!(
                sources.contains_key(preference.source_id.as_str()),
                "Preference references a missing source"
            );
            ensure!(
                preference_ids.insert(&preference.source_id),
                "Duplicate reference preference"
            );
        }
        for step in &self.steps {
            ensure!(
                !step.id.trim().is_empty() && !step.title.trim().is_empty(),
                "Step ID and title are required"
            );
            for id in step
                .parent_id
                .iter()
                .chain(&step.prerequisites)
                .chain(&step.next)
            {
                ensure!(
                    nodes.contains_key(id.as_str()),
                    "Step {} references missing step {id}",
                    step.id
                );
                ensure!(id != &step.id, "Step {} references itself", step.id);
            }
            let mut ancestors = HashSet::new();
            let mut current = Some(step.id.as_str());
            while let Some(id) = current {
                ensure!(ancestors.insert(id), "Cyclic parent hierarchy at {id}");
                current = nodes[id].parent_id.as_deref();
            }
            for anchor in &step.anchors {
                let source = sources
                    .get(anchor.source_id.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Unknown source {}", anchor.source_id))?;
                if let (Some(start), Some(end)) = (anchor.start_ms, anchor.end_ms) {
                    ensure!(start < end, "Invalid interval on step {}", step.id);
                }
                if let Some(duration) = source.duration_ms {
                    ensure!(
                        anchor.start_ms.is_none_or(|t| t < duration),
                        "Anchor starts beyond source duration"
                    );
                    ensure!(
                        anchor.end_ms.is_none_or(|t| t <= duration),
                        "Anchor ends beyond source duration"
                    );
                }
            }
        }
        // Prerequisites must be acyclic. Route edges may cycle for backtracking.
        fn visit<'a>(
            id: &'a str,
            nodes: &HashMap<&'a str, &'a Step>,
            active: &mut HashSet<&'a str>,
            done: &mut HashSet<&'a str>,
        ) -> Result<()> {
            if done.contains(id) {
                return Ok(());
            }
            ensure!(active.insert(id), "Cyclic prerequisites at {id}");
            for prerequisite in &nodes[id].prerequisites {
                visit(prerequisite, nodes, active, done)?;
            }
            active.remove(id);
            done.insert(id);
            Ok(())
        }
        let (mut active, mut done) = (HashSet::new(), HashSet::new());
        for step in &self.steps {
            visit(&step.id, &nodes, &mut active, &mut done)?;
        }
        Ok(())
    }

    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.id == id)
    }

    pub fn source_enabled(&self, id: &str) -> bool {
        self.sources.iter().any(|s| s.id == id)
            && self
                .reference_preferences
                .iter()
                .find(|p| p.source_id == id)
                .is_none_or(|p| p.enabled)
    }

    pub fn step_enabled(&self, step: &Step) -> bool {
        step.anchors.is_empty()
            || step
                .anchors
                .iter()
                .any(|a| self.source_enabled(&a.source_id))
    }
}

/// Preserve heading structure without claiming an LLM interpreted the source.
pub fn import_markdown(game: &str, title: &str, text: &str) -> Result<Guide> {
    ensure!(!text.trim().is_empty(), "The source text is empty");
    let source_id = "text-1".to_owned();
    let mut steps: Vec<Step> = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    for line in text.lines() {
        let level = line.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&level) && line.as_bytes().get(level) == Some(&b' ') {
            while stack.last().is_some_and(|(l, _)| *l >= level) {
                stack.pop();
            }
            let heading = line[level..].trim().to_owned();
            let id = format!("step-{}", steps.len() + 1);
            steps.push(Step {
                id: id.clone(),
                parent_id: stack.last().map(|(_, id)| id.clone()),
                title: heading.clone(),
                instructions: String::new(),
                prerequisites: vec![],
                next: vec![],
                completion_signals: vec![],
                anchors: vec![Anchor {
                    source_id: source_id.clone(),
                    start_ms: None,
                    end_ms: None,
                    section: Some(heading),
                }],
                optional: false,
            });
            stack.push((level, id));
        } else {
            if steps.is_empty() && !line.trim().is_empty() {
                steps.push(Step {
                    id: "step-1".into(),
                    parent_id: None,
                    title: title.into(),
                    instructions: String::new(),
                    prerequisites: vec![],
                    next: vec![],
                    completion_signals: vec![],
                    anchors: vec![Anchor {
                        source_id: source_id.clone(),
                        start_ms: None,
                        end_ms: None,
                        section: Some("Introduction".into()),
                    }],
                    optional: false,
                });
            }
            if let Some(step) = steps.last_mut() {
                step.instructions.push_str(line);
                step.instructions.push('\n');
            }
        }
    }
    for index in 0..steps.len().saturating_sub(1) {
        steps[index].next = vec![steps[index + 1].id.clone()];
    }
    for step in &mut steps {
        step.instructions = step.instructions.trim().to_owned();
    }
    let guide = Guide {
        schema_version: 1,
        id: uuid::Uuid::new_v4().to_string(),
        revision: 1,
        game: game.into(),
        title: title.into(),
        sources: vec![Source {
            id: source_id,
            title: title.into(),
            url: None,
            kind: SourceKind::Text,
            duration_ms: None,
            media_path: None,
            captions_path: None,
        }],
        steps,
        goals: vec![],
        reference_preferences: vec![],
        coverage_notes: vec![
            "Heading-based import. Prerequisites and completion signals have not been verified."
                .into(),
        ],
    };
    guide.validate()?;
    Ok(guide)
}

pub fn youtube_id(input: &str) -> Result<String> {
    let url = url::Url::parse(input.trim())?;
    ensure!(url.scheme() == "https", "YouTube URLs must use HTTPS");
    let host = url.host_str().unwrap_or_default();
    let id = match host {
        "youtu.be" => url.path().trim_matches('/').to_owned(),
        "youtube.com" | "www.youtube.com" | "m.youtube.com" => {
            if url.path() == "/watch" {
                url.query_pairs()
                    .find(|(k, _)| k == "v")
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_default()
            } else {
                let parts: Vec<_> = url.path().split('/').filter(|s| !s.is_empty()).collect();
                if parts.len() == 2 && ["shorts", "embed", "live"].contains(&parts[0]) {
                    parts[1].into()
                } else {
                    String::new()
                }
            }
        }
        _ => bail!("Expected a youtube.com or youtu.be URL"),
    };
    ensure!(
        id.len() == 11
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "Invalid YouTube video ID"
    );
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_preserves_hierarchy_and_content() {
        let g = import_markdown(
            "Example",
            "Guide",
            "# Town\nSpeak to the guard.\n## Gate\nFind the key.\n# Forest\nFollow the path.",
        )
        .unwrap();
        assert_eq!(g.steps.len(), 3);
        assert_eq!(g.steps[1].parent_id.as_deref(), Some("step-1"));
        assert!(g.steps[0].instructions.contains("guard"));
        assert_eq!(g.steps[2].parent_id, None);
    }
    #[test]
    fn graph_rejects_cycles_and_missing_references() {
        let mut g = import_markdown("Game", "Guide", "# A\n# B").unwrap();
        g.steps[0].prerequisites = vec!["step-2".into()];
        g.steps[1].prerequisites = vec!["step-1".into()];
        assert!(g.validate().is_err());
        g.steps[1].prerequisites.clear();
        g.steps[0].parent_id = Some("missing".into());
        assert!(g.validate().is_err());
    }
    #[test]
    fn youtube_urls_are_restricted() {
        assert_eq!(
            youtube_id("https://youtu.be/P05edoxsWQg?t=12").unwrap(),
            "P05edoxsWQg"
        );
        assert!(youtube_id("https://youtube.com.evil.test/watch?v=P05edoxsWQg").is_err());
        assert!(youtube_id("file:///tmp/video").is_err());
    }
}
