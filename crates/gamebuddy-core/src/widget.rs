use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WidgetContent {
    Text {
        body: String,
    },
    Checklist {
        items: Vec<String>,
    },
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Image {
        url: String,
        caption: String,
    },
    Video {
        url: String,
        start_seconds: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Widget {
    pub id: String,
    pub title: String,
    pub content: WidgetContent,
    #[serde(default)]
    pub source_step_ids: Vec<String>,
    /// Per-row/checklist-item references override the widget-wide references.
    #[serde(default)]
    pub item_sources: Vec<WidgetItemSources>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WidgetItemSources {
    /// Zero-based table row or checklist item index.
    pub item_index: usize,
    pub source_step_ids: Vec<String>,
}

impl Widget {
    pub fn validate_sources(&self, guide: &crate::Guide) -> Result<()> {
        self.validate()?;
        ensure!(
            self.source_step_ids
                .iter()
                .chain(self.item_sources.iter().flat_map(|s| &s.source_step_ids))
                .all(|id| guide.step(id).is_some_and(|s| guide.step_enabled(s))),
            "Widget references an unknown or disabled source step"
        );
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.id.is_empty()
                && self.id.len() <= 80
                && self
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c)),
            "Widget ID must be a short identifier"
        );
        ensure!(
            !["current-step", "walkthrough-outline", "location-map"].contains(&self.id.as_str()),
            "This widget is managed by GameBuddy"
        );
        ensure!(
            !self.title.trim().is_empty() && self.title.len() <= 200,
            "Widget needs a short title"
        );
        ensure!(
            serde_json::to_vec(self)?.len() <= 32_000,
            "Widget is too large"
        );
        let item_count = match &self.content {
            WidgetContent::Checklist { items } => items.len(),
            WidgetContent::Table { rows, .. } => rows.len(),
            _ => 0,
        };
        let mut indices = std::collections::HashSet::new();
        ensure!(
            self.item_sources
                .iter()
                .all(|s| s.item_index < item_count && indices.insert(s.item_index)),
            "Item provenance must address a distinct existing row or checklist item"
        );
        match &self.content {
            WidgetContent::Table { columns, rows } => {
                ensure!(
                    !columns.is_empty() && columns.len() <= 12 && rows.len() <= 100,
                    "Table dimensions are invalid"
                );
                ensure!(
                    rows.iter().all(|row| row.len() == columns.len()),
                    "Table rows must match the column count"
                );
            }
            WidgetContent::Image { url, .. } => {
                let u = url::Url::parse(url)?;
                ensure!(
                    u.scheme() == "https" && u.username().is_empty() && u.password().is_none(),
                    "Map/image widgets require a public HTTPS source URL"
                );
            }
            WidgetContent::Video { url, .. } => {
                crate::youtube_id(url)?;
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn per_item_provenance_rejects_unknown_disabled_and_out_of_range_references() {
        let mut guide = crate::import_markdown("Game", "Walkthrough", "# Start\nGo east").unwrap();
        let mut widget: Widget = serde_json::from_value(serde_json::json!({
            "id":"route", "title":"Route", "content":{"kind":"checklist","items":["Go east","My goal"]},
            "source_step_ids":["step-1"],
            "item_sources":[{"item_index":0,"source_step_ids":["step-1"]},{"item_index":1,"source_step_ids":[]}]
        })).unwrap();
        widget.validate_sources(&guide).unwrap();
        widget.item_sources[0].source_step_ids[0] = "invented".into();
        assert!(widget.validate_sources(&guide).is_err());
        widget.item_sources[0].source_step_ids[0] = "step-1".into();
        widget.item_sources[0].item_index = 2;
        assert!(widget.validate_sources(&guide).is_err());
        widget.item_sources[0].item_index = 1;
        assert!(widget.validate_sources(&guide).is_err());
        widget.item_sources[0].item_index = 0;
        guide
            .reference_preferences
            .push(crate::ReferencePreference {
                source_id: guide.sources[0].id.clone(),
                enabled: false,
                preference: String::new(),
            });
        assert!(widget.validate_sources(&guide).is_err());
    }

    #[test]
    fn rejects_invalid_tables_and_executable_urls() {
        let mut w = Widget {
            id: "items".into(),
            title: "Items".into(),
            source_step_ids: vec![],
            item_sources: vec![],
            content: WidgetContent::Table {
                columns: vec!["Item".into()],
                rows: vec![vec![]],
            },
        };
        assert!(w.validate().is_err());
        w.content = WidgetContent::Image {
            url: "javascript:alert(1)".into(),
            caption: "".into(),
        };
        assert!(w.validate().is_err());
        w.content = WidgetContent::Text {
            body: "<script>displayed as text</script>".into(),
        };
        assert!(w.validate().is_ok());
    }
}
