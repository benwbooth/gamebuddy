use super::*;
use crate::conversation::{Message, game_key};

pub(super) fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS conversation_messages(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            game TEXT NOT NULL, session_id TEXT NOT NULL, message_id TEXT NOT NULL,
            role TEXT NOT NULL CHECK(role IN ('user','assistant')),
            text TEXT NOT NULL, at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(game,session_id,message_id)
        );
        CREATE INDEX IF NOT EXISTS conversation_game_id ON conversation_messages(game,id);",
    )?;
    Ok(())
}

impl Store {
    pub fn has_player_conversation(&self, game: &str) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversation_messages WHERE game=?1 AND role='user')",
            [game_key(game)],
            |row| row.get(0),
        )?)
    }
    /// Each completed transcript is committed immediately, independently of game
    /// progress, guide revisions, capture sessions and provider process lifetime.
    pub fn append_conversation(
        &self,
        game: &str,
        session: &str,
        message_id: &str,
        role: &str,
        text: &str,
    ) -> Result<()> {
        ensure!(
            !game_key(game).is_empty(),
            "Conversation needs a game identity"
        );
        ensure!(
            matches!(role, "user" | "assistant"),
            "Invalid conversation role"
        );
        if text.trim().is_empty() {
            return Ok(());
        }
        self.connection.execute(
            "INSERT INTO conversation_messages(game,session_id,message_id,role,text)
             VALUES(?1,?2,?3,?4,?5) ON CONFLICT(game,session_id,message_id) DO NOTHING",
            params![game_key(game), session, message_id, role, text],
        )?;
        Ok(())
    }

    /// Newest matching messages, returned in conversational order. `before`
    /// paginates older history; a query is a literal substring, never SQL/LIKE.
    pub fn conversation_messages(
        &self,
        game: &str,
        query: &str,
        before: Option<i64>,
        limit: usize,
    ) -> Result<Vec<Message>> {
        let mut stmt = self.connection.prepare(
            "SELECT id,session_id,role,text,at FROM conversation_messages
             WHERE game=?1 AND (?2='' OR instr(lower(text),lower(?2))>0)
               AND (?3 IS NULL OR id<?3) ORDER BY id DESC LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            params![game_key(game), query, before, limit.clamp(1, 50) as i64],
            |r| {
                Ok(Message {
                    id: r.get(0)?,
                    session_id: r.get(1)?,
                    role: r.get(2)?,
                    text: r.get(3)?,
                    at: r.get(4)?,
                    truncated: false,
                })
            },
        )?;
        let mut messages = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        messages.reverse();
        Ok(messages)
    }

    /// Import the old guide-scoped text-chat tail once, preserving it when a
    /// different guide for the same game is selected. Old voice had no archive.
    pub fn migrate_conversation(&self, game: &str) -> Result<()> {
        let marker = format!("conversation-migrated:{}", game_key(game));
        if self.get::<bool>(&marker)?.unwrap_or(false) {
            return Ok(());
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        if !self.get::<bool>(&marker)?.unwrap_or(false) {
            // Ascending creation order keeps the latest guide's history last.
            for guide in self
                .guides()?
                .into_iter()
                .rev()
                .filter(|g| game_key(&g.game) == game_key(game))
            {
                let history: Vec<serde_json::Value> = self
                    .get(&format!("conversation:{}", guide.id))?
                    .unwrap_or_default();
                for (i, turn) in history.iter().enumerate() {
                    for (field, role) in [("player", "user"), ("answer", "assistant")] {
                        if let Some(text) = turn[field].as_str() {
                            self.append_conversation(
                                game,
                                &format!("legacy:{}", guide.id),
                                &format!("{i}:{role}"),
                                role,
                                text,
                            )?;
                        }
                    }
                }
            }
            self.put(&marker, &true)?;
        }
        tx.commit()?;
        Ok(())
    }
}
