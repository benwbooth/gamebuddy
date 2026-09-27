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
}

impl Widget {
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
            self.id != "current-step",
            "The current-step widget is managed by GameBuddy"
        );
        ensure!(
            !self.title.trim().is_empty() && self.title.len() <= 200,
            "Widget needs a short title"
        );
        ensure!(
            serde_json::to_vec(self)?.len() <= 32_000,
            "Widget is too large"
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
    fn rejects_invalid_tables_and_executable_urls() {
        let mut w = Widget {
            id: "items".into(),
            title: "Items".into(),
            source_step_ids: vec![],
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
