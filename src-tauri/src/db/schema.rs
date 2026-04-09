use rusqlite::Connection;

use crate::util::errors::AppError;

pub fn apply(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS imports (
          id INTEGER PRIMARY KEY,
          source_root TEXT NOT NULL,
          started_at TEXT NOT NULL,
          finished_at TEXT NULL,
          status TEXT NOT NULL,
          files_scanned INTEGER NOT NULL DEFAULT 0,
          files_added INTEGER NOT NULL DEFAULT 0,
          files_updated INTEGER NOT NULL DEFAULT 0,
          files_deleted INTEGER NOT NULL DEFAULT 0,
          assets_added INTEGER NOT NULL DEFAULT 0,
          assets_updated INTEGER NOT NULL DEFAULT 0,
          assets_deleted INTEGER NOT NULL DEFAULT 0,
          notes TEXT NULL
        );

        CREATE TABLE IF NOT EXISTS file_entries (
          id INTEGER PRIMARY KEY,
          path TEXT NOT NULL UNIQUE,
          parent_path TEXT NOT NULL,
          filename TEXT NOT NULL,
          extension TEXT NULL,
          detected_format TEXT NULL,
          mime_type TEXT NULL,
          file_size INTEGER NOT NULL,
          mtime_utc TEXT NOT NULL,
          ctime_utc TEXT NULL,
          quick_hash BLOB NULL,
          full_hash BLOB NULL,
          json_kind TEXT NULL,
          is_deleted INTEGER NOT NULL DEFAULT 0,
          last_seen_import_id INTEGER NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_file_entries_filename ON file_entries(filename);
        CREATE INDEX IF NOT EXISTS idx_file_entries_mtime ON file_entries(mtime_utc);
        CREATE INDEX IF NOT EXISTS idx_file_entries_last_seen_import_id ON file_entries(last_seen_import_id);
        CREATE INDEX IF NOT EXISTS idx_file_entries_full_hash ON file_entries(full_hash);

        CREATE TABLE IF NOT EXISTS assets (
          id INTEGER PRIMARY KEY,
          primary_file_id INTEGER NOT NULL,
          media_kind TEXT NOT NULL,
          display_type TEXT NOT NULL,
          title TEXT NULL,
          taken_at_utc TEXT NULL,
          taken_at_local TEXT NULL,
          timezone_hint TEXT NULL,
          width INTEGER NULL,
          height INTEGER NULL,
          duration_ms INTEGER NULL,
          orientation INTEGER NULL,
          gps_lat REAL NULL,
          gps_lon REAL NULL,
          gps_alt REAL NULL,
          camera_make TEXT NULL,
          camera_model TEXT NULL,
          is_favorite INTEGER NOT NULL DEFAULT 0,
          is_deleted INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_assets_taken_at_utc ON assets(taken_at_utc);
        CREATE INDEX IF NOT EXISTS idx_assets_media_kind ON assets(media_kind);
        CREATE INDEX IF NOT EXISTS idx_assets_primary_file_id ON assets(primary_file_id);
        CREATE INDEX IF NOT EXISTS idx_assets_deleted_taken_at ON assets(is_deleted, taken_at_utc);

        CREATE TABLE IF NOT EXISTS asset_files (
          asset_id INTEGER NOT NULL,
          file_id INTEGER NOT NULL,
          role TEXT NOT NULL,
          PRIMARY KEY (asset_id, file_id, role)
        );
        CREATE INDEX IF NOT EXISTS idx_asset_files_file_id ON asset_files(file_id);
        CREATE INDEX IF NOT EXISTS idx_asset_files_asset_role ON asset_files(asset_id, role);

        CREATE TABLE IF NOT EXISTS albums (
          id INTEGER PRIMARY KEY,
          name TEXT NOT NULL,
          source_path TEXT NOT NULL UNIQUE,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_albums_name ON albums(name);

        CREATE TABLE IF NOT EXISTS album_assets (
          album_id INTEGER NOT NULL,
          asset_id INTEGER NOT NULL,
          position_hint INTEGER NULL,
          added_at TEXT NOT NULL,
          PRIMARY KEY (album_id, asset_id)
        );
        CREATE INDEX IF NOT EXISTS idx_album_assets_asset_id ON album_assets(asset_id);
        CREATE INDEX IF NOT EXISTS idx_album_assets_album_position ON album_assets(album_id, position_hint);

        CREATE TABLE IF NOT EXISTS sidecar_metadata (
          id INTEGER PRIMARY KEY,
          asset_id INTEGER NOT NULL UNIQUE,
          sidecar_file_id INTEGER NULL,
          json_raw TEXT NULL,
          photo_taken_time_utc TEXT NULL,
          geo_lat REAL NULL,
          geo_lon REAL NULL,
          geo_alt REAL NULL,
          people_json TEXT NULL,
          google_photos_origin TEXT NULL,
          google_photos_url TEXT NULL,
          import_version INTEGER NOT NULL DEFAULT 1,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS asset_relationships (
          src_asset_id INTEGER NOT NULL,
          dst_asset_id INTEGER NOT NULL,
          relation_type TEXT NOT NULL,
          PRIMARY KEY (src_asset_id, dst_asset_id, relation_type)
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS search_fts USING fts5(
          asset_id UNINDEXED,
          filename,
          album_name,
          camera_model,
          taken_day
        );

        CREATE TABLE IF NOT EXISTS ingress_diagnostics (
          id INTEGER PRIMARY KEY,
          import_id INTEGER NOT NULL,
          severity TEXT NOT NULL,
          diagnostic_type TEXT NOT NULL,
          asset_id INTEGER NULL,
          file_id INTEGER NULL,
          related_path TEXT NULL,
          message TEXT NOT NULL,
          details_json TEXT NULL,
          created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_ingress_diag_import_id ON ingress_diagnostics(import_id);
        CREATE INDEX IF NOT EXISTS idx_ingress_diag_type ON ingress_diagnostics(diagnostic_type);
        CREATE INDEX IF NOT EXISTS idx_ingress_diag_asset_id ON ingress_diagnostics(asset_id);

        CREATE TABLE IF NOT EXISTS app_logs (
          id INTEGER PRIMARY KEY,
          created_at TEXT NOT NULL,
          level TEXT NOT NULL,
          scope TEXT NOT NULL,
          message TEXT NOT NULL,
          asset_id INTEGER NULL
        );
        CREATE INDEX IF NOT EXISTS idx_app_logs_created_at ON app_logs(created_at DESC);
        ",
    )?;

    ensure_column(conn, "sidecar_metadata", "google_photos_url", "TEXT NULL")?;

    Ok(())
}

  fn ensure_column(
    conn: &Connection,
    table_name: &str,
    column_name: &str,
    column_sql: &str,
  ) -> Result<(), AppError> {
    let pragma_sql = format!("PRAGMA table_info({table_name})");
    let mut stmt = conn.prepare(&pragma_sql)?;
    let existing = stmt
      .query_map([], |row| row.get::<_, String>(1))?
      .collect::<Result<Vec<_>, _>>()?;

    if existing.iter().any(|item| item == column_name) {
      return Ok(());
    }

    conn.execute(
      &format!("ALTER TABLE {table_name} ADD COLUMN {column_name} {column_sql}"),
      [],
    )?;
    Ok(())
  }
