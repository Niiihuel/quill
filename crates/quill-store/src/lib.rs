//! quill's durable record (spec.md D5): the events each session showed, in order, as JSON.
//! The engine appends an event before the UI sees it and replays them all at start.
//! Streaming deltas are left out; the finished text they make is kept.

use quill_model::{AgentEvent, Event, SessionId};
use rusqlite::{params, Connection};
use std::path::Path;

pub struct Store {
    conn: Connection,
}

pub type Result<T> = rusqlite::Result<T>;

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS events (
                 id INTEGER PRIMARY KEY,
                 session INTEGER NOT NULL,
                 event TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS events_by_session ON events (session, id);",
        )?;
        Ok(Self { conn })
    }

    /// Whether `event` is worth keeping: what the transcript shows once it has settled.
    pub fn keeps(event: &Event) -> bool {
        match event {
            Event::Status { .. }
            | Event::WorkingTree { .. }
            | Event::Deleted { .. }
            | Event::Reading { .. }
            | Event::FileRead { .. }
            | Event::FileWriteFailed { .. } => false,
            Event::Agent { event, .. } => !matches!(
                event,
                AgentEvent::TextDelta { .. }
                    | AgentEvent::Thinking { .. }
                    | AgentEvent::Tokens { .. }
                    | AgentEvent::Running { .. }
                    | AgentEvent::Reply { .. }
            ),
            _ => true,
        }
    }

    pub fn append(&self, session: SessionId, event: &Event) -> Result<()> {
        if !Self::keeps(event) {
            return Ok(());
        }
        let json = serde_json::to_string(event).expect("events serialize");
        self.conn.execute(
            "INSERT INTO events (session, event) VALUES (?1, ?2)",
            params![session as i64, json],
        )?;
        Ok(())
    }

    /// Forget every event of `session`.
    pub fn delete(&self, session: SessionId) -> Result<()> {
        self.conn.execute(
            "DELETE FROM events WHERE session = ?1",
            params![session as i64],
        )?;
        Ok(())
    }

    /// Every session with its events, oldest session first.
    pub fn load(&self) -> Result<Vec<(SessionId, Vec<Event>)>> {
        let mut statement = self
            .conn
            .prepare("SELECT session, event FROM events ORDER BY session, id")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)? as SessionId, row.get::<_, String>(1)?))
        })?;
        let mut sessions: Vec<(SessionId, Vec<Event>)> = vec![];
        for row in rows {
            let (session, json) = row?;
            // An event this version cannot read is skipped, never a reason to lose the rest.
            let Ok(event) = serde_json::from_str(&json) else {
                continue;
            };
            match sessions.last_mut() {
                Some((id, events)) if *id == session => events.push(event),
                _ => sessions.push((session, vec![event])),
            }
        }
        Ok(sessions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quill_model::Usage;

    #[test]
    fn sessions_come_back_in_order_without_deltas() {
        let store = Store::in_memory().unwrap();
        let created = Event::SessionCreated {
            session: 1,
            agent: "claude".into(),
            cwd: "/work".into(),
        };
        store.append(1, &created).unwrap();
        store
            .append(
                1,
                &Event::Queued {
                    session: 1,
                    request: "r".into(),
                    text: "hi".into(),
                },
            )
            .unwrap();
        store
            .append(
                1,
                &Event::Agent {
                    session: 1,
                    request: None,
                    event: AgentEvent::TextDelta {
                        text: "o".into(),
                        parent: None,
                    },
                },
            )
            .unwrap();
        let finished = Event::Agent {
            session: 1,
            request: Some("r".into()),
            event: AgentEvent::TurnFinished {
                ok: true,
                interrupted: false,
                usage: Usage::default(),
                error: None,
            },
        };
        store.append(1, &finished).unwrap();
        store
            .append(
                2,
                &Event::SessionCreated {
                    session: 2,
                    agent: "claude".into(),
                    cwd: "/b".into(),
                },
            )
            .unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].1.len(), 3);
        assert_eq!(loaded[0].1[0], created);
        assert_eq!(loaded[0].1[2], finished);
    }

    #[test]
    fn a_deleted_session_does_not_come_back() {
        let store = Store::in_memory().unwrap();
        for session in [1, 2] {
            let created = Event::SessionCreated {
                session,
                agent: "claude".into(),
                cwd: "/work".into(),
            };
            store.append(session, &created).unwrap();
        }
        store.delete(1).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(
            loaded.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![2]
        );
    }
}
