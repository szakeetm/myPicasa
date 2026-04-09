mod queries;
mod schema;

use std::{path::Path, sync::Mutex};

use rusqlite::Connection;

pub use queries::DatabaseQueries;

use crate::util::errors::AppError;

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn new(path: &Path) -> Result<Self, AppError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        schema::apply(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn with_connection<T, F>(&self, work: F) -> Result<T, AppError>
    where
        F: FnOnce(&Connection) -> Result<T, AppError>,
    {
        let guard = self.conn.lock().expect("database mutex poisoned");
        work(&guard)
    }

    pub fn reset(&self) -> Result<(), AppError> {
        let guard = self.conn.lock().expect("database mutex poisoned");
        guard.execute_batch(
            "
            PRAGMA foreign_keys = OFF;
            DELETE FROM album_assets;
            DELETE FROM asset_relationships;
            DELETE FROM asset_files;
            DELETE FROM sidecar_metadata;
            DELETE FROM ingress_diagnostics;
            DELETE FROM search_fts;
            DELETE FROM assets;
            DELETE FROM file_entries;
            DELETE FROM albums;
            DELETE FROM imports;
            DELETE FROM app_logs;
            DELETE FROM viewer_video_transcodes;
            DELETE FROM sqlite_sequence;
            PRAGMA foreign_keys = ON;
            ",
        )?;
        schema::apply(&guard)?;
        Ok(())
    }

    pub fn clear_diagnostics(&self) -> Result<(), AppError> {
        let guard = self.conn.lock().expect("database mutex poisoned");
        guard.execute("DELETE FROM ingress_diagnostics", [])?;
        Ok(())
    }

    pub fn clear_logs(&self) -> Result<(), AppError> {
        let guard = self.conn.lock().expect("database mutex poisoned");
        guard.execute("DELETE FROM app_logs", [])?;
        Ok(())
    }

    pub fn clear_logs_by_scope(&self, scopes: &[&str]) -> Result<(), AppError> {
        let guard = self.conn.lock().expect("database mutex poisoned");
        if scopes.is_empty() {
            return Ok(());
        }
        let placeholders = vec!["?"; scopes.len()].join(", ");
        let sql = format!("DELETE FROM app_logs WHERE scope IN ({placeholders})");
        let params = scopes.iter().map(|scope| scope.to_string()).collect::<Vec<_>>();
        guard.execute(&sql, rusqlite::params_from_iter(params.iter()))?;
        Ok(())
    }
}
