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
}
