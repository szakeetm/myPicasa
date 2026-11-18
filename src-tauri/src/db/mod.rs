mod queries;
mod schema;

use std::{fs, path::Path, sync::Mutex};

use rusqlite::{Connection, MAIN_DB, OpenFlags, backup::Backup, params};

pub use queries::DatabaseQueries;

use crate::util::errors::AppError;

pub struct Database {
    conn: Mutex<Connection>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remap_replaces_only_the_longest_leading_root_once() {
        let db = Database::new(Path::new(":memory:")).unwrap();
        db.with_connection(|conn| {
            conn.execute_batch(
                "INSERT INTO albums (name, source_path, created_at, updated_at) VALUES
                    ('nested', '/Photos/Trip/Photos', '', ''),
                    ('repeated', '/Photos/copy/Photos', '', ''),
                    ('sibling', '/Photos-old', '', ''),
                    ('other', '/Moved/other', '', '');
                 INSERT INTO imports (source_root, started_at, status) VALUES
                    ('/Photos; /Moved; /Photos-old', '', 'completed');",
            )?;
            Ok(())
        })
        .unwrap();
        db.remap_takeout_roots(&[
            ("/Photos/".into(), "/Moved".into()),
            ("/Photos/Trip".into(), "/Trip".into()),
            ("/Moved".into(), "/Next".into()),
        ])
        .unwrap();
        db.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT source_path FROM albums ORDER BY id")?;
            let paths = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            assert_eq!(
                paths,
                [
                    "/Trip/Photos",
                    "/Moved/copy/Photos",
                    "/Photos-old",
                    "/Next/other"
                ]
            );
            let roots: String =
                conn.query_row("SELECT source_root FROM imports", [], |row| row.get(0))?;
            assert_eq!(roots, "/Moved; /Next; /Photos-old");
            Ok(())
        })
        .unwrap();
    }
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

    pub fn export_to(&self, path: &Path) -> Result<(), AppError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if path.exists() {
            fs::remove_file(path)?;
        }
        let guard = self.conn.lock().expect("database mutex poisoned");
        guard.pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
        guard.backup(MAIN_DB, path, None)?;
        Ok(())
    }

    pub fn import_from(&self, path: &Path) -> Result<(), AppError> {
        let mut guard = self.conn.lock().expect("database mutex poisoned");
        guard.restore(MAIN_DB, path, None::<fn(rusqlite::backup::Progress)>)?;
        Ok(())
    }

    pub fn stage_backup(source: &Path, staged: &Path) -> Result<Self, AppError> {
        let source = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let check: String = source.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
        if check != "ok" {
            return Err(AppError::Message(format!(
                "Invalid backup database: {check}"
            )));
        }
        for table in ["file_entries", "assets", "albums", "imports"] {
            if !source.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1)",
                [table],
                |row| row.get::<_, bool>(0),
            )? {
                return Err(AppError::Message(format!(
                    "Backup database is missing {table}"
                )));
            }
        }
        let mut conn = Connection::open(staged)?;
        if Backup::new(&source, &mut conn)?.step(-1)? != rusqlite::backup::StepResult::Done {
            return Err(AppError::Message(
                "Could not copy the backup database".into(),
            ));
        }
        schema::apply(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn remap_takeout_roots(&self, mappings: &[(String, String)]) -> Result<(), AppError> {
        let mut mappings: Vec<_> = mappings
            .iter()
            .map(|(from, to)| (from.replace('\\', "/"), to.replace('\\', "/")))
            .collect();
        // A nested configured root owns its subtree even when its destination is unchanged.
        mappings.sort_by_key(|(from, _)| std::cmp::Reverse(from.trim_end_matches('/').len()));
        let remap = |path: &str| {
            for (from, to) in &mappings {
                let root = from.trim_end_matches('/');
                if from.is_empty() {
                    continue;
                }
                if let Some(suffix) = path.strip_prefix(root) {
                    if suffix.is_empty() || suffix.starts_with('/') {
                        return if suffix.is_empty() {
                            to.clone()
                        } else {
                            format!("{}{suffix}", to.trim_end_matches('/'))
                        };
                    }
                }
            }
            path.to_owned()
        };
        self.with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            for (table, column) in [("file_entries", "path"), ("albums", "source_path")] {
                let mut stmt = tx.prepare(&format!("SELECT id, {column} FROM {table}"))?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?;
                let mut moved = Vec::new();
                for row in rows {
                    let (id, path) = row?;
                    let next = remap(&path);
                    if next != path {
                        moved.push((id, next));
                    }
                }
                let mut update = tx.prepare(&format!(
                    "UPDATE {table} SET {column} = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1"
                ))?;
                // Free unique keys together so mappings can swap roots.
                let prefix: String =
                    tx.query_row("SELECT hex(randomblob(16))", [], |row| row.get(0))?;
                for (id, _) in &moved {
                    update.execute(params![id, format!("{prefix}/{id}")])?;
                }
                for (id, next) in moved {
                    update.execute(params![id, next])?;
                }
            }
            for (table, column) in [("file_entries", "parent_path"), ("imports", "source_root")] {
                let mut stmt = tx.prepare(&format!("SELECT id, {column} FROM {table}"))?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?;
                let mut changed = Vec::new();
                for row in rows {
                    let (id, value) = row?;
                    let next = if table == "imports" {
                        value
                            .split("; ")
                            .map(|root| remap(&root.replace('\\', "/")))
                            .collect::<Vec<_>>()
                            .join("; ")
                    } else {
                        remap(&value)
                    };
                    if next != value {
                        changed.push((id, next));
                    }
                }
                let mut update =
                    tx.prepare(&format!("UPDATE {table} SET {column} = ?2 WHERE id = ?1"))?;
                for (id, next) in changed {
                    update.execute(params![id, next])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
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
        let params = scopes
            .iter()
            .map(|scope| scope.to_string())
            .collect::<Vec<_>>();
        guard.execute(&sql, rusqlite::params_from_iter(params.iter()))?;
        Ok(())
    }
}
