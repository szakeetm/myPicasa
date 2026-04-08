use std::path::Path;

use rusqlite::{params, OptionalExtension};

use crate::{
    models::{
        AlbumSummary, AssetDetail, AssetListItem, AssetListRequest, AssetListResponse,
        DiagnosticEntry, FileScanRecord, ImportProgress, LogEntry, ParsedSidecar,
    },
    util::{errors::AppError, time::utc_now},
};

pub trait DatabaseQueries {
    fn insert_log(
        &self,
        level: &str,
        scope: &str,
        message: &str,
        asset_id: Option<i64>,
    ) -> Result<(), AppError>;
    fn create_import(&self, source_root: &str) -> Result<i64, AppError>;
    fn finish_import(&self, import: &ImportProgress) -> Result<(), AppError>;
    fn upsert_file_entry(&self, import_id: i64, scan: &FileScanRecord) -> Result<i64, AppError>;
    fn soft_delete_missing_files(&self, import_id: i64, roots: &[String]) -> Result<u32, AppError>;
    fn upsert_album(&self, source_path: &str) -> Result<i64, AppError>;
    fn upsert_asset_for_file(
        &self,
        file_id: i64,
        scan: &FileScanRecord,
        sidecar: Option<&ParsedSidecar>,
    ) -> Result<i64, AppError>;
    fn attach_asset_file(&self, asset_id: i64, file_id: i64, role: &str) -> Result<(), AppError>;
    fn attach_album_asset(&self, album_id: i64, asset_id: i64) -> Result<(), AppError>;
    fn set_sidecar_metadata(
        &self,
        asset_id: i64,
        sidecar_file_id: Option<i64>,
        sidecar: &ParsedSidecar,
    ) -> Result<(), AppError>;
    fn replace_search_row(&self, asset_id: i64) -> Result<(), AppError>;
    fn add_diagnostic(
        &self,
        import_id: i64,
        severity: &str,
        diagnostic_type: &str,
        related_path: Option<&str>,
        message: &str,
    ) -> Result<(), AppError>;
    fn resolve_sidecar_target(&self, sidecar_path: &str) -> Result<Option<(i64, i64)>, AppError>;
    fn list_albums(&self) -> Result<Vec<AlbumSummary>, AppError>;
    fn list_assets_by_date(&self, request: AssetListRequest) -> Result<AssetListResponse, AppError>;
    fn list_assets_by_album(
        &self,
        album_id: i64,
        request: AssetListRequest,
    ) -> Result<AssetListResponse, AppError>;
    fn search_assets(&self, request: AssetListRequest) -> Result<AssetListResponse, AppError>;
    fn get_asset_detail(&self, asset_id: i64) -> Result<AssetDetail, AppError>;
    fn get_live_photo_pair(&self, asset_id: i64) -> Result<Option<String>, AppError>;
    fn get_ingress_diagnostics(&self) -> Result<Vec<DiagnosticEntry>, AppError>;
    fn get_recent_logs(&self, limit: u32) -> Result<Vec<LogEntry>, AppError>;
}

impl DatabaseQueries for super::Database {
    fn insert_log(
        &self,
        level: &str,
        scope: &str,
        message: &str,
        asset_id: Option<i64>,
    ) -> Result<(), AppError> {
        self.with_connection(|conn| {
            conn.execute(
                "INSERT INTO app_logs (created_at, level, scope, message, asset_id) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![utc_now(), level, scope, message, asset_id],
            )?;
            Ok(())
        })
    }

    fn create_import(&self, source_root: &str) -> Result<i64, AppError> {
        self.with_connection(|conn| {
            conn.execute(
                "INSERT INTO imports (source_root, started_at, status) VALUES (?1, ?2, 'running')",
                params![source_root, utc_now()],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    fn finish_import(&self, import: &ImportProgress) -> Result<(), AppError> {
        self.with_connection(|conn| {
            conn.execute(
                "UPDATE imports
                 SET finished_at = ?2, status = ?3, files_scanned = ?4, files_added = ?5, files_updated = ?6,
                     files_deleted = ?7, assets_added = ?8, assets_updated = ?9, assets_deleted = ?10, notes = ?11
                 WHERE id = ?1",
                params![
                    import.import_id,
                    utc_now(),
                    import.status,
                    import.files_scanned,
                    import.files_added,
                    import.files_updated,
                    import.files_deleted,
                    import.assets_added,
                    import.assets_updated,
                    import.assets_deleted,
                    import.message
                ],
            )?;
            Ok(())
        })
    }

    fn upsert_file_entry(&self, import_id: i64, scan: &FileScanRecord) -> Result<i64, AppError> {
        self.with_connection(|conn| {
            let existing = conn
                .query_row(
                    "SELECT id, file_size, mtime_utc FROM file_entries WHERE path = ?1",
                    params![scan.path],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()?;

            let now = utc_now();

            if let Some((id, old_size, old_mtime)) = existing {
                conn.execute(
                    "UPDATE file_entries
                     SET parent_path = ?2, filename = ?3, extension = ?4, detected_format = ?5, mime_type = ?6,
                         file_size = ?7, mtime_utc = ?8, ctime_utc = ?9, quick_hash = ?10, json_kind = ?11,
                         is_deleted = 0, last_seen_import_id = ?12, updated_at = ?13
                     WHERE id = ?1",
                    params![
                        id,
                        scan.parent_path,
                        scan.filename,
                        scan.extension,
                        scan.detected_format,
                        scan.mime_type,
                        scan.file_size,
                        scan.mtime_utc,
                        scan.ctime_utc,
                        scan.quick_hash,
                        scan.json_kind,
                        import_id,
                        now
                    ],
                )?;
                if old_size != scan.file_size || old_mtime != scan.mtime_utc {
                    self.insert_log("debug", "import.file", &format!("updated {}", scan.path), None)?;
                }
                Ok(id)
            } else {
                conn.execute(
                    "INSERT INTO file_entries
                     (path, parent_path, filename, extension, detected_format, mime_type, file_size, mtime_utc, ctime_utc,
                      quick_hash, full_hash, json_kind, is_deleted, last_seen_import_id, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11, 0, ?12, ?13, ?13)",
                    params![
                        scan.path,
                        scan.parent_path,
                        scan.filename,
                        scan.extension,
                        scan.detected_format,
                        scan.mime_type,
                        scan.file_size,
                        scan.mtime_utc,
                        scan.ctime_utc,
                        scan.quick_hash,
                        scan.json_kind,
                        import_id,
                        now
                    ],
                )?;
                Ok(conn.last_insert_rowid())
            }
        })
    }

    fn soft_delete_missing_files(&self, import_id: i64, roots: &[String]) -> Result<u32, AppError> {
        self.with_connection(|conn| {
            let mut count = 0_u32;
            for root in roots {
                count += conn.execute(
                    "UPDATE file_entries SET is_deleted = 1, updated_at = ?1
                     WHERE last_seen_import_id != ?2 AND is_deleted = 0 AND path LIKE ?3",
                    params![utc_now(), import_id, format!("{root}%")],
                )? as u32;
            }
            Ok(count)
        })
    }

    fn upsert_album(&self, source_path: &str) -> Result<i64, AppError> {
        self.with_connection(|conn| {
            let existing = conn
                .query_row(
                    "SELECT id FROM albums WHERE source_path = ?1",
                    params![source_path],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            if let Some(id) = existing {
                conn.execute(
                    "UPDATE albums SET name = ?2, updated_at = ?3 WHERE id = ?1",
                    params![id, file_name(source_path), utc_now()],
                )?;
                Ok(id)
            } else {
                conn.execute(
                    "INSERT INTO albums (name, source_path, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
                    params![file_name(source_path), source_path, utc_now()],
                )?;
                Ok(conn.last_insert_rowid())
            }
        })
    }

    fn upsert_asset_for_file(
        &self,
        file_id: i64,
        scan: &FileScanRecord,
        sidecar: Option<&ParsedSidecar>,
    ) -> Result<i64, AppError> {
        self.with_connection(|conn| {
            let existing = conn
                .query_row(
                    "SELECT asset_id FROM asset_files WHERE file_id = ?1 AND role IN ('primary', 'original', 'edited') LIMIT 1",
                    params![file_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;

            let taken_at = sidecar
                .and_then(|item| item.photo_taken_time_utc.clone())
                .unwrap_or_else(|| scan.mtime_utc.clone());
            let title = scan.filename.clone();

            if let Some(asset_id) = existing {
                conn.execute(
                    "UPDATE assets
                     SET media_kind = ?2, title = ?3, taken_at_utc = ?4, updated_at = ?5
                     WHERE id = ?1",
                    params![asset_id, scan.candidate_type, title, taken_at, utc_now()],
                )?;
                Ok(asset_id)
            } else {
                conn.execute(
                    "INSERT INTO assets
                     (primary_file_id, media_kind, display_type, title, taken_at_utc, taken_at_local, timezone_hint,
                      width, height, duration_ms, orientation, gps_lat, gps_lon, gps_alt, camera_make, camera_model,
                      is_favorite, is_deleted, created_at, updated_at)
                     VALUES (?1, ?2, 'original', ?3, ?4, NULL, NULL, NULL, NULL, NULL, NULL, ?5, ?6, ?7, NULL, NULL, 0, 0, ?8, ?8)",
                    params![
                        file_id,
                        scan.candidate_type,
                        title,
                        taken_at,
                        sidecar.and_then(|item| item.geo_lat),
                        sidecar.and_then(|item| item.geo_lon),
                        sidecar.and_then(|item| item.geo_alt),
                        utc_now()
                    ],
                )?;
                Ok(conn.last_insert_rowid())
            }
        })
    }

    fn attach_asset_file(&self, asset_id: i64, file_id: i64, role: &str) -> Result<(), AppError> {
        self.with_connection(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO asset_files (asset_id, file_id, role) VALUES (?1, ?2, ?3)",
                params![asset_id, file_id, role],
            )?;
            Ok(())
        })
    }

    fn attach_album_asset(&self, album_id: i64, asset_id: i64) -> Result<(), AppError> {
        self.with_connection(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO album_assets (album_id, asset_id, position_hint, added_at) VALUES (?1, ?2, NULL, ?3)",
                params![album_id, asset_id, utc_now()],
            )?;
            Ok(())
        })
    }

    fn set_sidecar_metadata(
        &self,
        asset_id: i64,
        sidecar_file_id: Option<i64>,
        sidecar: &ParsedSidecar,
    ) -> Result<(), AppError> {
        self.with_connection(|conn| {
            conn.execute(
                "INSERT INTO sidecar_metadata
                 (asset_id, sidecar_file_id, json_raw, photo_taken_time_utc, geo_lat, geo_lon, geo_alt,
                  people_json, google_photos_origin, import_version, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?10)
                 ON CONFLICT(asset_id) DO UPDATE SET
                   sidecar_file_id = excluded.sidecar_file_id,
                   json_raw = excluded.json_raw,
                   photo_taken_time_utc = excluded.photo_taken_time_utc,
                   geo_lat = excluded.geo_lat,
                   geo_lon = excluded.geo_lon,
                   geo_alt = excluded.geo_alt,
                   people_json = excluded.people_json,
                   google_photos_origin = excluded.google_photos_origin,
                   updated_at = excluded.updated_at",
                params![
                    asset_id,
                    sidecar_file_id,
                    sidecar.json_raw,
                    sidecar.photo_taken_time_utc,
                    sidecar.geo_lat,
                    sidecar.geo_lon,
                    sidecar.geo_alt,
                    sidecar.people_json,
                    sidecar.google_photos_origin,
                    utc_now()
                ],
            )?;
            Ok(())
        })
    }

    fn replace_search_row(&self, asset_id: i64) -> Result<(), AppError> {
        self.with_connection(|conn| {
            let row = conn.query_row(
                "SELECT a.id, f.filename, COALESCE(group_concat(DISTINCT al.name), ''), COALESCE(a.camera_model, ''), COALESCE(substr(a.taken_at_utc, 1, 10), '')
                 FROM assets a
                 JOIN file_entries f ON f.id = a.primary_file_id
                 LEFT JOIN album_assets aa ON aa.asset_id = a.id
                 LEFT JOIN albums al ON al.id = aa.album_id
                 WHERE a.id = ?1
                 GROUP BY a.id, f.filename, a.camera_model, a.taken_at_utc",
                params![asset_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            );

            if let Ok((id, filename, album_name, camera_model, taken_day)) = row {
                conn.execute("DELETE FROM search_fts WHERE asset_id = ?1", params![id])?;
                conn.execute(
                    "INSERT INTO search_fts (asset_id, filename, album_name, camera_model, taken_day) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![id, filename, album_name, camera_model, taken_day],
                )?;
            }

            Ok(())
        })
    }

    fn add_diagnostic(
        &self,
        import_id: i64,
        severity: &str,
        diagnostic_type: &str,
        related_path: Option<&str>,
        message: &str,
    ) -> Result<(), AppError> {
        self.with_connection(|conn| {
            conn.execute(
                "INSERT INTO ingress_diagnostics
                 (import_id, severity, diagnostic_type, related_path, message, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![import_id, severity, diagnostic_type, related_path, message, utc_now()],
            )?;
            Ok(())
        })
    }

    fn resolve_sidecar_target(&self, sidecar_path: &str) -> Result<Option<(i64, i64)>, AppError> {
        let path = Path::new(sidecar_path);
        let Some(stem) = path.file_stem().and_then(|item| item.to_str()) else {
            return Ok(None);
        };
        let candidate_prefix = stem.trim_end_matches(".supplemental-metadata");
        self.with_connection(|conn| {
            conn.query_row(
                "SELECT a.id, f.id
                 FROM file_entries f
                 JOIN asset_files af ON af.file_id = f.id
                 JOIN assets a ON a.id = af.asset_id
                 WHERE f.parent_path = ?1
                   AND f.filename LIKE ?2
                   AND f.is_deleted = 0
                 LIMIT 1",
                params![
                    path.parent().and_then(|item| item.to_str()).unwrap_or(""),
                    format!("{candidate_prefix}.%")
                ],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(AppError::from)
        })
    }

    fn list_albums(&self) -> Result<Vec<AlbumSummary>, AppError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT al.id, al.name, al.source_path, COUNT(aa.asset_id)
                 FROM albums al
                 LEFT JOIN album_assets aa ON aa.album_id = al.id
                 GROUP BY al.id, al.name, al.source_path
                 ORDER BY al.name COLLATE NOCASE",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(AlbumSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    source_path: row.get(2)?,
                    asset_count: row.get(3)?,
                })
            })?;
            Ok(rows.filter_map(Result::ok).collect())
        })
    }

    fn list_assets_by_date(&self, request: AssetListRequest) -> Result<AssetListResponse, AppError> {
        paged_asset_query(
            self,
            "SELECT a.id, a.title, a.media_kind, a.taken_at_utc, f.path, COALESCE(group_concat(DISTINCT al.name), '')
             FROM assets a
             JOIN file_entries f ON f.id = a.primary_file_id
             LEFT JOIN album_assets aa ON aa.asset_id = a.id
             LEFT JOIN albums al ON al.id = aa.album_id
             WHERE a.is_deleted = 0 AND a.media_kind IN ('photo', 'video', 'live_photo')",
            " ORDER BY COALESCE(a.taken_at_utc, f.mtime_utc) DESC, a.id DESC",
            request,
        )
    }

    fn list_assets_by_album(
        &self,
        album_id: i64,
        request: AssetListRequest,
    ) -> Result<AssetListResponse, AppError> {
        paged_asset_query(
            self,
            &format!(
                "SELECT a.id, a.title, a.media_kind, a.taken_at_utc, f.path, COALESCE(group_concat(DISTINCT al.name), '')
                 FROM assets a
                 JOIN album_assets aa_filter ON aa_filter.asset_id = a.id AND aa_filter.album_id = {album_id}
                 JOIN file_entries f ON f.id = a.primary_file_id
                 LEFT JOIN album_assets aa ON aa.asset_id = a.id
                 LEFT JOIN albums al ON al.id = aa.album_id
                 WHERE a.is_deleted = 0 AND a.media_kind IN ('photo', 'video', 'live_photo')"
            ),
            " ORDER BY COALESCE(a.taken_at_utc, f.mtime_utc) DESC, a.id DESC",
            request,
        )
    }

    fn search_assets(&self, request: AssetListRequest) -> Result<AssetListResponse, AppError> {
        let query = request.query.clone().unwrap_or_default();
        self.with_connection(|conn| {
            let offset = request.cursor.unwrap_or_default();
            let limit = request.limit.unwrap_or(200) as i64;
            let mut stmt = conn.prepare(
                "SELECT a.id, a.title, a.media_kind, a.taken_at_utc, f.path, COALESCE(group_concat(DISTINCT al.name), '')
                 FROM search_fts s
                 JOIN assets a ON a.id = s.asset_id
                 JOIN file_entries f ON f.id = a.primary_file_id
                 LEFT JOIN album_assets aa ON aa.asset_id = a.id
                 LEFT JOIN albums al ON al.id = aa.album_id
                 WHERE search_fts MATCH ?1 AND a.is_deleted = 0 AND a.media_kind IN ('photo', 'video', 'live_photo')
                 GROUP BY a.id, a.title, a.media_kind, a.taken_at_utc, f.path
                 ORDER BY rank
                 LIMIT ?2 OFFSET ?3",
            )?;
            let rows = stmt.query_map(params![query, limit, offset], |row| map_asset_list_item(row))?;
            let items: Vec<_> = rows.filter_map(Result::ok).collect();
            Ok(AssetListResponse {
                items,
                next_cursor: Some(offset + limit as u32),
            })
        })
    }

    fn get_asset_detail(&self, asset_id: i64) -> Result<AssetDetail, AppError> {
        self.with_connection(|conn| {
            let detail = conn.query_row(
                "SELECT a.id, a.title, a.media_kind, a.display_type, a.taken_at_utc, a.width, a.height, a.duration_ms,
                        a.gps_lat, a.gps_lon, f.path, COALESCE(group_concat(DISTINCT al.name), '')
                 FROM assets a
                 JOIN file_entries f ON f.id = a.primary_file_id
                 LEFT JOIN album_assets aa ON aa.asset_id = a.id
                 LEFT JOIN albums al ON al.id = aa.album_id
                 WHERE a.id = ?1
                 GROUP BY a.id, a.title, a.media_kind, a.display_type, a.taken_at_utc, a.width, a.height, a.duration_ms,
                          a.gps_lat, a.gps_lon, f.path",
                params![asset_id],
                |row| {
                    Ok(AssetDetail {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        media_kind: row.get(2)?,
                        display_type: row.get(3)?,
                        taken_at_utc: row.get(4)?,
                        width: row.get(5)?,
                        height: row.get(6)?,
                        duration_ms: row.get(7)?,
                        gps_lat: row.get(8)?,
                        gps_lon: row.get(9)?,
                        primary_path: row.get(10)?,
                        albums: split_csv(row.get::<_, String>(11)?),
                        live_photo_video_path: None,
                    })
                },
            )?;

            let live_photo = conn
                .query_row(
                    "SELECT f.path
                     FROM asset_files af
                     JOIN file_entries f ON f.id = af.file_id
                     WHERE af.asset_id = ?1 AND af.role = 'live_photo_video'
                     LIMIT 1",
                    params![asset_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;

            Ok(AssetDetail {
                live_photo_video_path: live_photo,
                ..detail
            })
        })
    }

    fn get_live_photo_pair(&self, asset_id: i64) -> Result<Option<String>, AppError> {
        self.with_connection(|conn| {
            conn.query_row(
                "SELECT f.path
                 FROM asset_files af
                 JOIN file_entries f ON f.id = af.file_id
                 WHERE af.asset_id = ?1 AND af.role = 'live_photo_video'
                 LIMIT 1",
                params![asset_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(AppError::from)
        })
    }

    fn get_ingress_diagnostics(&self) -> Result<Vec<DiagnosticEntry>, AppError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, import_id, severity, diagnostic_type, related_path, message, created_at
                 FROM ingress_diagnostics
                 ORDER BY id DESC
                 LIMIT 500",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(DiagnosticEntry {
                    id: row.get(0)?,
                    import_id: row.get(1)?,
                    severity: row.get(2)?,
                    diagnostic_type: row.get(3)?,
                    related_path: row.get(4)?,
                    message: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?;
            Ok(rows.filter_map(Result::ok).collect())
        })
    }

    fn get_recent_logs(&self, limit: u32) -> Result<Vec<LogEntry>, AppError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, created_at, level, scope, message, asset_id
                 FROM app_logs
                 ORDER BY id DESC
                 LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], |row| {
                Ok(LogEntry {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    level: row.get(2)?,
                    scope: row.get(3)?,
                    message: row.get(4)?,
                    asset_id: row.get(5)?,
                })
            })?;
            Ok(rows.filter_map(Result::ok).collect())
        })
    }
}

fn paged_asset_query(
    db: &super::Database,
    base_sql: &str,
    order_sql: &str,
    request: AssetListRequest,
) -> Result<AssetListResponse, AppError> {
    db.with_connection(|conn| {
        let offset = request.cursor.unwrap_or_default();
        let limit = request.limit.unwrap_or(200) as i64;
        let mut sql = format!("{base_sql} ");
        let mut filters = Vec::<String>::new();

        if let Some(kind) = request.media_kind.as_deref() {
            filters.push(format!("a.media_kind = '{}'", kind.replace('\'', "''")));
        }
        if let Some(start) = request.date_from.as_deref() {
            filters.push(format!("a.taken_at_utc >= '{}'", start.replace('\'', "''")));
        }
        if let Some(end) = request.date_to.as_deref() {
            filters.push(format!("a.taken_at_utc <= '{}'", end.replace('\'', "''")));
        }
        if let Some(query) = request.query.as_deref() {
            let escaped = query.replace('\'', "''");
            filters.push(format!("(a.title LIKE '%{escaped}%' OR f.filename LIKE '%{escaped}%')"));
        }
        if !filters.is_empty() {
            sql.push_str(" AND ");
            sql.push_str(&filters.join(" AND "));
        }
        sql.push_str(" GROUP BY a.id, a.title, a.media_kind, a.taken_at_utc, f.path");
        sql.push_str(order_sql);
        sql.push_str(&format!(" LIMIT {limit} OFFSET {offset}"));

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| map_asset_list_item(row))?;
        let items = rows.filter_map(Result::ok).collect::<Vec<_>>();
        Ok(AssetListResponse {
            items,
            next_cursor: Some(offset + limit as u32),
        })
    })
}

fn map_asset_list_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssetListItem> {
    Ok(AssetListItem {
        id: row.get(0)?,
        title: row.get(1)?,
        media_kind: row.get(2)?,
        taken_at_utc: row.get(3)?,
        primary_path: row.get(4)?,
        albums: split_csv(row.get::<_, String>(5)?),
    })
}

fn split_csv(value: String) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|item| item.to_str())
        .unwrap_or(path)
        .to_string()
}
