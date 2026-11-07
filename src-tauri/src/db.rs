//! Per-project SQLite database powering the Database panel.
//!
//! Every project gets its own `app.db`. The panel is a real SQL console, and
//! the generated app can read the same file through the exported schema.

use anyhow::{Context, Result};
use rusqlite::types::ValueRef;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub affected: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub name: String,
    pub columns: Vec<ColumnInfo>,
    pub rows: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub kind: String,
    pub notnull: bool,
    pub pk: bool,
}

fn open(project_id: &str) -> Result<rusqlite::Connection> {
    let dir = crate::project::dir(project_id);
    std::fs::create_dir_all(&dir)?;
    let conn = rusqlite::Connection::open(dir.join("app.db")).context("opening database")?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    Ok(conn)
}

fn cell(v: ValueRef<'_>) -> serde_json::Value {
    match v {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(i) => serde_json::json!(i),
        ValueRef::Real(f) => serde_json::json!(f),
        ValueRef::Text(t) => serde_json::json!(String::from_utf8_lossy(t)),
        ValueRef::Blob(b) => serde_json::json!(format!("<{} bytes>", b.len())),
    }
}

/// `SELECT`/`PRAGMA`/`WITH` return rows; everything else reports rows affected.
pub fn query(project_id: &str, sql: &str) -> Result<QueryResult> {
    let conn = open(project_id)?;
    let trimmed = sql.trim().trim_end_matches(';').trim();
    if trimmed.is_empty() {
        return Ok(QueryResult {
            columns: vec![],
            rows: vec![],
            affected: 0,
            message: "Nothing to run.".into(),
        });
    }

    let first = trimmed
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_uppercase();
    let returns_rows = matches!(first.as_str(), "SELECT" | "PRAGMA" | "WITH" | "EXPLAIN" | "VALUES");

    if returns_rows {
        let mut stmt = conn.prepare(trimmed).context("preparing query")?;
        let columns: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let n = columns.len();
        let mut rows_out = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let mut row = Vec::with_capacity(n);
            for i in 0..n {
                row.push(cell(r.get_ref(i)?));
            }
            rows_out.push(row);
            if rows_out.len() >= 2000 {
                break; // keep the UI responsive on huge tables
            }
        }
        let count = rows_out.len();
        Ok(QueryResult {
            columns,
            rows: rows_out,
            affected: 0,
            message: format!("{count} row{}", if count == 1 { "" } else { "s" }),
        })
    } else {
        // execute_batch handles multi-statement scripts (schema files, seeds).
        if trimmed.contains(';') {
            conn.execute_batch(trimmed).context("running script")?;
            return Ok(QueryResult {
                columns: vec![],
                rows: vec![],
                affected: 0,
                message: "Script executed.".into(),
            });
        }
        let affected = conn.execute(trimmed, []).context("running statement")?;
        Ok(QueryResult {
            columns: vec![],
            rows: vec![],
            affected,
            message: format!("{affected} row{} affected", if affected == 1 { "" } else { "s" }),
        })
    }
}

pub fn tables(project_id: &str) -> Result<Vec<TableInfo>> {
    let conn = open(project_id)?;
    let names: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let mut out = Vec::new();
    for name in names {
        let mut columns = Vec::new();
        {
            let mut stmt = conn.prepare(&format!("PRAGMA table_info(\"{}\")", name.replace('"', "")))?;
            let rows = stmt.query_map([], |r| {
                Ok(ColumnInfo {
                    name: r.get::<_, String>(1)?,
                    kind: r.get::<_, String>(2).unwrap_or_default(),
                    notnull: r.get::<_, i64>(3).unwrap_or(0) == 1,
                    pk: r.get::<_, i64>(5).unwrap_or(0) > 0,
                })
            })?;
            for c in rows.flatten() {
                columns.push(c);
            }
        }
        let rows: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM \"{}\"", name.replace('"', "")), [], |r| r.get(0))
            .unwrap_or(0);
        out.push(TableInfo { name, columns, rows });
    }
    Ok(out)
}

/// Ready-made schemas so the panel is useful in one click.
pub const SEEDS: &[(&str, &str, &str)] = &[
    (
        "users",
        "Users + sessions",
        r#"CREATE TABLE IF NOT EXISTS users (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  email TEXT NOT NULL UNIQUE,
  name TEXT,
  avatar_url TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE IF NOT EXISTS sessions (
  token TEXT PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
INSERT OR IGNORE INTO users (email, name) VALUES
  ('ada@example.com','Ada Lovelace'),
  ('grace@example.com','Grace Hopper');"#,
    ),
    (
        "items",
        "Generic items list",
        r#"CREATE TABLE IF NOT EXISTS items (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  title TEXT NOT NULL,
  body TEXT,
  done INTEGER NOT NULL DEFAULT 0,
  position INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_items_done ON items(done, position);
INSERT INTO items (title, body) VALUES
  ('Try the simulator','Rotate it with the orientation toggle'),
  ('Add a feature','Describe it in chat and watch it appear');"#,
    ),
    (
        "scores",
        "Game scores + runs",
        r#"CREATE TABLE IF NOT EXISTS scores (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  player TEXT NOT NULL,
  score INTEGER NOT NULL,
  level INTEGER NOT NULL DEFAULT 1,
  duration_ms INTEGER,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_scores_top ON scores(score DESC);
INSERT INTO scores (player, score, level) VALUES ('you', 0, 1);"#,
    ),
    (
        "content",
        "Posts + tags",
        r#"CREATE TABLE IF NOT EXISTS posts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  slug TEXT NOT NULL UNIQUE,
  title TEXT NOT NULL,
  body TEXT,
  status TEXT NOT NULL DEFAULT 'draft',
  published_at TEXT
);
CREATE TABLE IF NOT EXISTS tags (
  post_id INTEGER NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
  tag TEXT NOT NULL,
  PRIMARY KEY (post_id, tag)
);
INSERT OR IGNORE INTO posts (slug, title, status) VALUES ('hello','Hello world','published');"#,
    ),
];

pub fn seed(project_id: &str, kind: &str) -> Result<String> {
    let (_, label, sql) = SEEDS
        .iter()
        .find(|(k, ..)| *k == kind)
        .ok_or_else(|| anyhow::anyhow!("Unknown schema '{kind}'"))?;
    let conn = open(project_id)?;
    conn.execute_batch(sql).context("applying schema")?;
    Ok(format!("Created the {label} schema."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> String {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        std::fs::create_dir_all(crate::project::files_dir(&id)).unwrap();
        crate::project::save(&crate::project::Project { id: id.clone(), ..Default::default() })
            .unwrap();
        id
    }

    #[test]
    fn ddl_dml_and_select_all_report_sensibly() {
        let id = scratch();
        let r = query(&id, "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, score REAL)").unwrap();
        assert_eq!(r.affected, 0);

        let r = query(&id, "INSERT INTO t (name, score) VALUES ('a', 1.5)").unwrap();
        assert_eq!(r.affected, 1);
        assert!(r.message.contains("1 row affected"));

        let r = query(&id, "SELECT id, name, score FROM t").unwrap();
        assert_eq!(r.columns, vec!["id", "name", "score"]);
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0][1], serde_json::json!("a"));
        assert_eq!(r.rows[0][2], serde_json::json!(1.5));
        assert_eq!(r.message, "1 row");

        // NULLs must survive the JSON round trip as null, not as "".
        query(&id, "INSERT INTO t (name) VALUES (NULL)").unwrap();
        let r = query(&id, "SELECT name FROM t WHERE name IS NULL").unwrap();
        assert_eq!(r.rows[0][0], serde_json::Value::Null);

        let _ = std::fs::remove_dir_all(crate::project::dir(&id));
    }

    #[test]
    fn empty_and_broken_sql_fail_gracefully() {
        let id = scratch();
        assert_eq!(query(&id, "   ").unwrap().message, "Nothing to run.");
        let err = query(&id, "SELEKT * FROM nope").unwrap_err().to_string();
        assert!(!err.is_empty());
        let _ = std::fs::remove_dir_all(crate::project::dir(&id));
    }

    #[test]
    fn every_seed_schema_applies_and_is_introspectable() {
        for (kind, label, _) in SEEDS {
            let id = scratch();
            let msg = seed(&id, kind).unwrap();
            assert!(msg.contains(label), "{kind}: {msg}");

            let t = tables(&id).unwrap();
            assert!(!t.is_empty(), "{kind} created no tables");
            for tbl in &t {
                assert!(!tbl.columns.is_empty(), "{}.{} has no columns", kind, tbl.name);
                assert!(
                    tbl.columns.iter().any(|c| c.pk),
                    "{}.{} has no primary key",
                    kind,
                    tbl.name
                );
            }
            // seeds are idempotent — applying twice must not error
            seed(&id, kind).unwrap();
            let _ = std::fs::remove_dir_all(crate::project::dir(&id));
        }
    }

    #[test]
    fn unknown_seed_is_rejected() {
        let id = scratch();
        assert!(seed(&id, "not-a-schema").is_err());
        let _ = std::fs::remove_dir_all(crate::project::dir(&id));
    }

    #[test]
    fn multi_statement_scripts_run_as_a_batch() {
        let id = scratch();
        let r = query(
            &id,
            "CREATE TABLE a (x INTEGER); CREATE TABLE b (y TEXT); INSERT INTO a VALUES (1);",
        )
        .unwrap();
        assert_eq!(r.message, "Script executed.");
        let names: Vec<String> = tables(&id).unwrap().into_iter().map(|t| t.name).collect();
        assert!(names.contains(&"a".to_string()) && names.contains(&"b".to_string()));
        let _ = std::fs::remove_dir_all(crate::project::dir(&id));
    }

    #[test]
    fn table_introspection_reports_row_counts_and_keys() {
        let id = scratch();
        seed(&id, "items").unwrap();
        let t = tables(&id).unwrap();
        let items = t.iter().find(|x| x.name == "items").unwrap();
        assert_eq!(items.rows, 2, "seed rows should be present");
        let id_col = items.columns.iter().find(|c| c.name == "id").unwrap();
        assert!(id_col.pk);
        let title = items.columns.iter().find(|c| c.name == "title").unwrap();
        assert!(title.notnull);
        let _ = std::fs::remove_dir_all(crate::project::dir(&id));
    }
}
