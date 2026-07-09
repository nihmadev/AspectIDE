//! Schema migration, row deserialisation, and shared helpers.

use std::str::FromStr;

use aspect_core::{AppError, AppResult};
use rusqlite::{Connection, Row};

use crate::model::{Memory, MemoryRelation, RelationKind};
use crate::store::SCHEMA_VERSION;

/// Idempotent v0 → v1 migration.
pub fn migrate(conn: &Connection) -> AppResult<()> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(to_service)?;
    if version >= SCHEMA_VERSION {
        return Ok(());
    }
    for stmt in [
        "ALTER TABLE memories ADD COLUMN superseded INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE memories ADD COLUMN forget_after INTEGER",
    ] {
        if let Err(error) = conn.execute(stmt, []) {
            if !is_duplicate_column(&error) {
                return Err(to_service(error));
            }
        }
    }
    conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))
        .map_err(to_service)?;
    Ok(())
}

pub(crate) fn is_duplicate_column(error: &rusqlite::Error) -> bool {
    error.to_string().contains("duplicate column name")
}

pub fn row_to_memory(row: &Row) -> rusqlite::Result<Memory> {
    let metadata_text: String = row.get(3)?;
    let metadata = serde_json::from_str(&metadata_text).unwrap_or_else(|_| serde_json::json!({}));
    Ok(Memory {
        id: row.get(0)?,
        category: row.get(1)?,
        content: row.get(2)?,
        metadata,
        importance: row.get(4)?,
        pinned: row.get::<_, i64>(5)? != 0,
        source: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        last_accessed_at: row.get(9)?,
        access_count: row.get(10)?,
        superseded: row.get::<_, i64>(11)? != 0,
        forget_after: row.get(12)?,
        has_embedding: row.get::<_, i64>(13)? != 0,
    })
}

pub fn row_to_relation(row: &Row) -> rusqlite::Result<MemoryRelation> {
    let relation_text: String = row.get(3)?;
    let relation = RelationKind::from_str(&relation_text).unwrap_or(RelationKind::Related);
    Ok(MemoryRelation {
        id: row.get(0)?,
        source_id: row.get(1)?,
        target_id: row.get(2)?,
        relation,
        confidence: row.get(4)?,
        created_at: row.get(5)?,
    })
}

pub fn collect<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&Row) -> rusqlite::Result<T>>,
) -> AppResult<Vec<T>> {
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(to_service)?);
    }
    Ok(out)
}

pub fn normalize_metadata(value: Option<serde_json::Value>) -> serde_json::Value {
    match value {
        Some(value) if value.is_object() => value,
        _ => serde_json::json!({}),
    }
}

pub fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn to_service<E: std::fmt::Display>(error: E) -> AppError {
    AppError::Service(error.to_string())
}
