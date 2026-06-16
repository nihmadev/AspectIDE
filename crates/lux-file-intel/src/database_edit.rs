use std::path::Path;

use lux_core::{
    AppError, AppResult, DatabaseColumnPreview, DatabaseTablePreview, FileInspectionOptions,
};
use rusqlite::{types::ValueRef, Connection, OpenFlags};
use serde::{Deserialize, Serialize};

const MAX_TABLES: usize = 64;
const MAX_COLUMNS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseExecuteRequest {
    pub sql: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseExecuteResult {
    pub rows_affected: usize,
    pub last_insert_rowid: i64,
    pub columns: Vec<String>,
    /// Cell is `None` for SQL NULL, `Some(_)` otherwise, so the editor can tell
    /// NULL apart from an empty-TEXT cell and preserve it on save.
    pub rows: Vec<Vec<Option<String>>>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseCellUpdate {
    pub table: String,
    pub rowid: i64,
    pub column: String,
    /// `None` writes SQL NULL; `Some("")` writes an empty TEXT cell. This split
    /// keeps NULL distinguishable from `''` on round-trip edits.
    pub value: Option<String>,
}

pub fn database_execute(path: &Path, sql: &str) -> AppResult<DatabaseExecuteResult> {
    let connection = open_writable(path)?;
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err(AppError::Service("SQL is empty".to_string()));
    }
    let mut statement = connection
        .prepare(trimmed)
        .map_err(|error| AppError::Service(error.to_string()))?;
    let column_count = statement.column_count();
    if column_count == 0 {
        let rows_affected = statement
            .execute([])
            .map_err(|error| AppError::Service(error.to_string()))?;
        let last_insert_rowid = connection.last_insert_rowid();
        return Ok(DatabaseExecuteResult {
            rows_affected,
            last_insert_rowid,
            columns: Vec::new(),
            rows: Vec::new(),
            message: format!("Statement OK. {rows_affected} row(s) affected."),
        });
    }

    let columns = (0..column_count)
        .map(|index| {
            statement
                .column_name(index)
                .map_or_else(|_| format!("column_{index}"), ToOwned::to_owned)
        })
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    let mut row_iter = statement
        .query([])
        .map_err(|error| AppError::Service(error.to_string()))?;
    while let Some(row) = row_iter
        .next()
        .map_err(|error| AppError::Service(error.to_string()))?
    {
        let mut values = Vec::with_capacity(column_count);
        for index in 0..column_count {
            values.push(sqlite_value_to_cell(
                row.get_ref(index)
                    .map_err(|error| AppError::Service(error.to_string()))?,
            ));
        }
        rows.push(values);
        if rows.len() >= 500 {
            break;
        }
    }
    let row_count = rows.len();
    Ok(DatabaseExecuteResult {
        rows_affected: row_count,
        last_insert_rowid: connection.last_insert_rowid(),
        columns,
        rows,
        message: format!("Query returned {row_count} row(s)."),
    })
}

pub fn database_update_cell(path: &Path, update: &DatabaseCellUpdate) -> AppResult<()> {
    let connection = open_writable(path)?;
    let quoted_table = quote_sqlite_ident(&update.table);
    let quoted_column = quote_sqlite_ident(&update.column);
    let sql = format!("UPDATE {quoted_table} SET {quoted_column} = ?1 WHERE rowid = ?2");
    let affected = connection
        .execute(&sql, rusqlite::params![update.value, update.rowid])
        .map_err(|error| AppError::Service(error.to_string()))?;
    if affected == 0 {
        return Err(AppError::Service(
            "no row matched the given rowid".to_string(),
        ));
    }
    Ok(())
}

pub fn database_tables(
    path: &Path,
    options: &FileInspectionOptions,
) -> AppResult<Vec<DatabaseTablePreview>> {
    if extension(path) == "duckdb" {
        return Err(AppError::Service(
            "DuckDB editing is not bundled; open the file externally.".to_string(),
        ));
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| AppError::Service(error.to_string()))?;
    let mut statement = connection
        .prepare(
            "SELECT name, type FROM sqlite_schema \
             WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' \
             ORDER BY type, name",
        )
        .map_err(|error| AppError::Service(error.to_string()))?;
    let schema_rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| AppError::Service(error.to_string()))?;

    let mut tables = Vec::new();
    for schema_row in schema_rows.take(MAX_TABLES) {
        let (name, kind) = schema_row.map_err(|error| AppError::Service(error.to_string()))?;
        let columns = table_columns(&connection, &name)?;
        let (rows, row_count, rows_truncated) = table_rows(&connection, &name, options)?;
        tables.push(DatabaseTablePreview {
            name,
            kind,
            columns,
            rows,
            row_count,
            truncated: rows_truncated,
        });
    }
    Ok(tables)
}

fn open_writable(path: &Path) -> AppResult<Connection> {
    if extension(path) == "duckdb" {
        return Err(AppError::Service(
            "DuckDB editing is not bundled.".to_string(),
        ));
    }
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .map_err(|error| AppError::Service(error.to_string()))
}

fn table_columns(connection: &Connection, table: &str) -> AppResult<Vec<DatabaseColumnPreview>> {
    let quoted = quote_sqlite_ident(table);
    let pragma = format!("PRAGMA table_info({quoted})");
    let mut statement = connection
        .prepare(&pragma)
        .map_err(|error| AppError::Service(error.to_string()))?;
    let rows = statement
        .query_map([], |row| {
            Ok(DatabaseColumnPreview {
                name: row.get(1)?,
                type_name: row.get(2)?,
                nullable: row.get::<_, i64>(3)? == 0,
                primary_key: row.get::<_, i64>(5)? != 0,
            })
        })
        .map_err(|error| AppError::Service(error.to_string()))?;
    rows.take(MAX_COLUMNS)
        .map(|row| row.map_err(|error| AppError::Service(error.to_string())))
        .collect()
}

fn table_rows(
    connection: &Connection,
    table: &str,
    options: &FileInspectionOptions,
) -> AppResult<(Vec<Vec<String>>, Option<usize>, bool)> {
    let quoted = quote_sqlite_ident(table);
    let sql = format!(
        "SELECT * FROM {quoted} LIMIT {}",
        options.max_rows.saturating_add(1)
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| AppError::Service(error.to_string()))?;
    let column_count = statement.column_count();
    let mut rows = Vec::new();
    let mut row_iter = statement
        .query([])
        .map_err(|error| AppError::Service(error.to_string()))?;
    while let Some(row) = row_iter
        .next()
        .map_err(|error| AppError::Service(error.to_string()))?
    {
        let mut values = Vec::with_capacity(column_count);
        for index in 0..column_count {
            values.push(
                sqlite_value_to_cell(
                    row.get_ref(index)
                        .map_err(|error| AppError::Service(error.to_string()))?,
                )
                .unwrap_or_default(),
            );
        }
        rows.push(values);
        if rows.len() > options.max_rows {
            break;
        }
    }
    let truncated = rows.len() > options.max_rows;
    if truncated {
        rows.truncate(options.max_rows);
    }
    let row_count = rows.len();
    Ok((rows, Some(row_count), truncated))
}

fn sqlite_value_to_cell(value: ValueRef<'_>) -> Option<String> {
    match value {
        ValueRef::Null => None,
        ValueRef::Integer(number) => Some(number.to_string()),
        ValueRef::Real(number) => Some(number.to_string()),
        ValueRef::Text(text) => Some(String::from_utf8_lossy(text).into_owned()),
        ValueRef::Blob(bytes) => Some(format!("<blob {} bytes>", bytes.len())),
    }
}

fn quote_sqlite_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn extension(path: &Path) -> String {
    lux_core::file_extension_for_path(path)
}
