use clap::Parser;
use gamebuddy_services::{Store, harness};
use rmcp::{
    ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    schemars, tool, tool_router,
    transport::stdio,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "GameBuddy MCP harness over stdio")]
struct Args {
    #[arg(long)]
    data_dir: Option<PathBuf>,
}
#[derive(Clone)]
struct GameBuddy {
    root: PathBuf,
}
#[derive(Deserialize, schemars::JsonSchema)]
struct Search {
    query: String,
}
#[derive(Deserialize, schemars::JsonSchema)]
struct Show {
    widget: gamebuddy_core::Widget,
}
#[derive(Deserialize, schemars::JsonSchema)]
struct Remove {
    id: String,
}
#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
struct MapPreference {
    id: String,
    enabled: bool,
}
#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
struct ProgressReport {
    step_id: String,
    player_report: String,
    expected_revision: u64,
    epoch: u64,
}
#[derive(Deserialize, schemars::JsonSchema)]
struct Prepare {
    game: String,
    #[serde(default)]
    platform: String,
}
#[derive(Deserialize, schemars::JsonSchema)]
struct EditPlan {
    /// PlanEdit object: expected_revision, summary, optional upsert_steps,
    /// remove_step_ids, goals and reference_preferences. Read get_outline first.
    edit: gamebuddy_services::plan::PlanEdit,
}
#[derive(Deserialize, schemars::JsonSchema)]
struct RestorePlan {
    expected_revision: u64,
    restore_revision: u64,
}
#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
struct ReadReference {
    source_id: String,
    start_ms: u64,
    end_ms: u64,
}
#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
struct Reference {
    expected_revision: u64,
    /// text or youtube
    kind: String,
    title: Option<String>,
    content: Option<String>,
    url: Option<String>,
}

fn result(value: anyhow::Result<Value>) -> CallToolResult {
    match value {
        Ok(value) => CallToolResult::success(vec![ContentBlock::text(value.to_string())]),
        Err(error) => CallToolResult::error(vec![ContentBlock::text(format!("{error:#}"))]),
    }
}
#[tool_router(server_handler)]
impl GameBuddy {
    #[tool(
        description = "Record an explicit statement by the player that a task is done. Never infer this from advice, a walkthrough or room identity. Stored separately as player-reported progress. Read current context for revision and epoch first."
    )]
    fn report_step_complete(
        &self,
        Parameters(report): Parameters<ProgressReport>,
    ) -> CallToolResult {
        self.call("report_step_complete", json!(report))
    }
    #[tool(
        description = "Read automatically prepared rooms, maps, source bindings and coverage. Map coordinates and task completion are not inferred from room identity."
    )]
    fn get_atlas(&self) -> CallToolResult {
        self.call("get_atlas", json!({}))
    }

    #[tool(
        description = "Add a player-supplied or source-backed HTTPS map to the active game. Never invent URLs."
    )]
    fn add_map(&self, Parameters(map): Parameters<gamebuddy_core::MapAsset>) -> CallToolResult {
        self.call("add_map", json!(map))
    }

    #[tool(
        description = "Enable or disable an available reference map according to player preference."
    )]
    fn set_map_enabled(&self, Parameters(args): Parameters<MapPreference>) -> CallToolResult {
        self.call("set_map_enabled", json!(args))
    }
    #[tool(
        description = "Read timestamped captions and imported text for an enabled reference in a range of up to ten minutes. Missing captions are returned empty, never invented."
    )]
    fn read_reference(&self, Parameters(args): Parameters<ReadReference>) -> CallToolResult {
        match serde_json::to_value(args) {
            Ok(value) => self.call("read_reference", value),
            Err(error) => result(Err(error.into())),
        }
    }
    #[tool(
        description = "Read the editable outline, personal goals, source preferences and revision history. Preserve stable IDs when editing."
    )]
    fn get_outline(&self) -> CallToolResult {
        self.call("get_outline", json!({}))
    }

    #[tool(
        description = "Adapt the outline and reference preferences to a player request. Supply a PlanEdit in edit: expected_revision, summary, optional upsert_steps (full Step objects), remove_step_ids, goals (id/description/step_ids), reference_preferences (source_id/enabled/preference). Atomic, revision checked, undoable. Never marks steps complete."
    )]
    fn edit_plan(&self, Parameters(args): Parameters<EditPlan>) -> CallToolResult {
        self.call("edit_plan", json!(args.edit))
    }

    #[tool(
        description = "Restore an earlier outline as a new revision. Does not revive stale completion evidence."
    )]
    fn restore_plan(&self, Parameters(args): Parameters<RestorePlan>) -> CallToolResult {
        self.call("restore_plan", json!({"expected_revision":args.expected_revision,"restore_revision":args.restore_revision}))
    }

    #[tool(
        description = "Attach a player-selected YouTube URL or pasted text reference to the active plan. kind is text or youtube. Text needs title/content; YouTube needs url and downloads media. Use edit_plan afterwards to reconcile routes and preferences. May take several minutes."
    )]
    async fn add_reference(&self, Parameters(args): Parameters<Reference>) -> CallToolResult {
        let root = self.root.clone();
        let task = tokio::task::spawn_blocking(move || {
            let store = Store::open(root)?;
            harness::call_tool(&store, "add_reference", serde_json::to_value(args)?)
        })
        .await;
        result(task.unwrap_or_else(|e| Err(e.into())))
    }

    fn call(&self, name: &str, args: Value) -> CallToolResult {
        result(Store::open(&self.root).and_then(|store| harness::call_tool(&store, name, args)))
    }
    #[tool(
        description = "Read GameBuddy's active game, guide sources, verified progress and visual location hypotheses. Observations include timestamps and can be stale; advice never proves completion."
    )]
    fn get_game_context(&self) -> CallToolResult {
        self.call("get_game_context", json!({}))
    }

    #[tool(
        description = "Search the active walkthrough for source-backed steps, instructions and video timestamps."
    )]
    fn search_guide(&self, Parameters(args): Parameters<Search>) -> CallToolResult {
        self.call("search_guide", json!({"query":args.query}))
    }

    #[tool(
        description = "Show or replace a draggable HTML OSD widget. Supply id, title, source_step_ids, content. Content kind: text/body; checklist/items; table/columns/rows; image/HTTPS url/caption; video/YouTube url/start_seconds. Use known source URLs only. No arbitrary script or HTML is accepted."
    )]
    fn show_widget(&self, Parameters(args): Parameters<Show>) -> CallToolResult {
        self.call("show_widget", json!({"widget":args.widget}))
    }

    #[tool(description = "Remove an assistant-created GameBuddy OSD widget.")]
    fn remove_widget(&self, Parameters(args): Parameters<Remove>) -> CallToolResult {
        self.call("remove_widget", json!({"id":args.id}))
    }

    #[tool(
        description = "Get the latest frame the user shared with GameBuddy, with its observation timestamp. Does not initiate screen capture or bypass desktop consent."
    )]
    fn get_latest_frame(&self) -> CallToolResult {
        let value = (|| -> anyhow::Result<_> {
            let store = Store::open(&self.root)?;
            let context = harness::call_tool(&store, "get_game_context", json!({}))?;
            let path = context["observation"]["last_frame"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("No frame has been shared"))?;
            let data = gamebuddy_services::image_base64(path)?;
            Ok((data, context["observation"]["estimate"].clone()))
        })();
        match value {
            Ok((data, estimate)) => CallToolResult::success(vec![
                ContentBlock::text(estimate.to_string()),
                ContentBlock::image(data, "image/jpeg"),
            ]),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(e.to_string())]),
        }
    }

    #[tool(
        description = "Prepare a game's guide automatically: reuse cached work, otherwise search YouTube, ask the configured reasoning model to choose a walkthrough, download it, import chapters and index visual references. This may take several minutes."
    )]
    async fn prepare_game(&self, Parameters(args): Parameters<Prepare>) -> CallToolResult {
        let root = self.root.clone();
        let task = tokio::task::spawn_blocking(move || {
            let store = Store::open(root)?;
            let guide = gamebuddy_services::prepare_game(&store,&args.game,&args.platform,&|message|eprintln!("{message}"))?;
            Ok(json!({"guide_id":guide.id,"game":guide.game,"title":guide.title,"nodes":guide.steps.len(),"coverage_notes":guide.coverage_notes}))
        }).await;
        result(task.unwrap_or_else(|e| Err(e.into())))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let root = args
        .data_dir
        .map(Ok)
        .unwrap_or_else(gamebuddy_services::data_dir)?;
    let service = GameBuddy { root }.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
