use std::{collections::HashMap, path::Path};

use rusqlite::{OptionalExtension, params};

use crate::{
    import::sidecar::takeout_match_score,
    media::thumb::probe_media_duration_ms,
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
    fn reconcile_assets_after_file_deletions(&self) -> Result<(Vec<i64>, Vec<i64>), AppError>;
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
    fn resolve_sidecar_target(
        &self,
        sidecar_path: &str,
        candidate_names: &[String],
    ) -> Result<Option<(i64, i64)>, AppError>;
    fn list_albums(&self) -> Result<Vec<AlbumSummary>, AppError>;
    fn list_assets_by_date(&self, request: AssetListRequest)
    -> Result<AssetListResponse, AppError>;
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
    fn get_logs_by_scope(&self, scopes: &[&str], limit: u32) -> Result<Vec<LogEntry>, AppError>;
    fn set_viewer_video_transcode_status(
        &self,
        asset_id: i64,
        status: &str,
        cache_path: Option<&str>,
    ) -> Result<(), AppError>;
    fn get_viewer_video_playback_statuses(
        &self,
        asset_ids: &[i64],
    ) -> Result<HashMap<i64, String>, AppError>;
    fn clear_viewer_video_transcode_statuses(&self) -> Result<(), AppError>;
    fn clear_viewer_video_transcode_statuses_for_assets(
        &self,
        asset_ids: &[i64],
    ) -> Result<(), AppError>;
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
        let (file_id, should_log_update) = self.with_connection(|conn| {
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
                Ok((id, old_size != scan.file_size || old_mtime != scan.mtime_utc))
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
                Ok((conn.last_insert_rowid(), false))
            }
        })?;

        if should_log_update {
            self.insert_log(
                "debug",
                "import.file",
                &format!("updated {}", scan.path),
                None,
            )?;
        }

        Ok(file_id)
    }

    fn soft_delete_missing_files(&self, import_id: i64, roots: &[String]) -> Result<u32, AppError> {
        self.with_connection(|conn| {
            let _ = roots;
            let count = conn.execute(
                "UPDATE file_entries
                 SET is_deleted = 1, updated_at = ?1
                 WHERE last_seen_import_id != ?2
                   AND is_deleted = 0",
                params![utc_now(), import_id],
            )? as u32;
            Ok(count)
        })
    }

    fn reconcile_assets_after_file_deletions(&self) -> Result<(Vec<i64>, Vec<i64>), AppError> {
        self.with_connection(|conn| {
            let mut stmt =
                conn.prepare("SELECT id, primary_file_id FROM assets WHERE is_deleted = 0")?;
            let assets = stmt
                .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;

            let now = utc_now();
            let mut deleted_asset_ids = Vec::new();
            let mut reindexed_asset_ids = Vec::new();

            for (asset_id, primary_file_id) in assets {
                let replacement = conn
                    .query_row(
                        "SELECT f.id, f.filename
                         FROM asset_files af
                         JOIN file_entries f ON f.id = af.file_id
                         WHERE af.asset_id = ?1
                           AND af.role != 'live_photo_video'
                           AND f.is_deleted = 0
                         ORDER BY CASE af.role
                             WHEN 'primary' THEN 0
                             WHEN 'original' THEN 1
                             WHEN 'edited' THEN 2
                             WHEN 'duplicate' THEN 3
                             ELSE 4
                         END,
                         f.id
                         LIMIT 1",
                        params![asset_id],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()?;

                if let Some((replacement_file_id, replacement_filename)) = replacement {
                    if replacement_file_id != primary_file_id {
                        conn.execute(
                            "UPDATE assets
                             SET primary_file_id = ?2, title = ?3, updated_at = ?4
                             WHERE id = ?1",
                            params![asset_id, replacement_file_id, replacement_filename, now],
                        )?;
                        reindexed_asset_ids.push(asset_id);
                    }
                    continue;
                }

                conn.execute(
                    "DELETE FROM search_fts WHERE asset_id = ?1",
                    params![asset_id],
                )?;
                conn.execute(
                    "DELETE FROM asset_relationships
                     WHERE src_asset_id = ?1 OR dst_asset_id = ?1",
                    params![asset_id],
                )?;
                conn.execute(
                    "UPDATE assets
                     SET is_deleted = 1, updated_at = ?2
                     WHERE id = ?1",
                    params![asset_id, now],
                )?;
                deleted_asset_ids.push(asset_id);
            }

            Ok((deleted_asset_ids, reindexed_asset_ids))
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
            let duration_ms = if scan.candidate_type == "video" {
                probe_media_duration_ms(Path::new(&scan.path))?
            } else {
                None
            };

            if let Some(asset_id) = existing {
                conn.execute(
                    "UPDATE assets
                     SET primary_file_id = ?2, media_kind = ?3, title = ?4, taken_at_utc = ?5,
                         duration_ms = ?6, is_deleted = 0, updated_at = ?7
                     WHERE id = ?1",
                    params![
                        asset_id,
                        file_id,
                        scan.candidate_type,
                        title,
                        taken_at,
                        duration_ms,
                        utc_now()
                    ],
                )?;
                Ok(asset_id)
            } else {
                conn.execute(
                    "INSERT INTO assets
                     (primary_file_id, media_kind, display_type, title, taken_at_utc, taken_at_local, timezone_hint,
                      width, height, duration_ms, orientation, gps_lat, gps_lon, gps_alt, camera_make, camera_model,
                      is_favorite, is_deleted, created_at, updated_at)
                     VALUES (?1, ?2, 'original', ?3, ?4, NULL, NULL, NULL, NULL, ?5, NULL, ?6, ?7, ?8, NULL, NULL, 0, 0, ?9, ?9)",
                    params![
                        file_id,
                        scan.candidate_type,
                        title,
                        taken_at,
                        duration_ms,
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
                                    people_json, google_photos_origin, google_photos_url, import_version, created_at, updated_at)
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, ?11)
                 ON CONFLICT(asset_id) DO UPDATE SET
                   sidecar_file_id = excluded.sidecar_file_id,
                   json_raw = excluded.json_raw,
                   photo_taken_time_utc = excluded.photo_taken_time_utc,
                   geo_lat = excluded.geo_lat,
                   geo_lon = excluded.geo_lon,
                   geo_alt = excluded.geo_alt,
                   people_json = excluded.people_json,
                   google_photos_origin = excluded.google_photos_origin,
                                     google_photos_url = excluded.google_photos_url,
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
                    sidecar.google_photos_url,
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
                params![
                    import_id,
                    severity,
                    diagnostic_type,
                    related_path,
                    message,
                    utc_now()
                ],
            )?;
            Ok(())
        })
    }

    fn resolve_sidecar_target(
        &self,
        sidecar_path: &str,
        candidate_names: &[String],
    ) -> Result<Option<(i64, i64)>, AppError> {
        let path = Path::new(sidecar_path);
        let Some(stem) = path.file_stem().and_then(|item| item.to_str()) else {
            return Ok(None);
        };
        let mut candidates = vec![stem.trim_end_matches(".supplemental-metadata").to_string()];
        for candidate in candidate_names {
            let trimmed = candidate.trim();
            if !trimmed.is_empty() && !candidates.iter().any(|existing| existing == trimmed) {
                candidates.push(trimmed.to_string());
            }
        }
        self.with_connection(|conn| {
            let parent_path = path.parent().and_then(|item| item.to_str()).unwrap_or("");
            let mut best_match: Option<(usize, i64, i64)> = None;

            for candidate in &candidates {
                let direct_match = conn
                    .query_row(
                        "SELECT a.id, f.id
                         FROM file_entries f
                         JOIN asset_files af ON af.file_id = f.id
                         JOIN assets a ON a.id = af.asset_id
                         WHERE f.parent_path = ?1
                           AND f.filename = ?2
                           AND f.is_deleted = 0
                         LIMIT 1",
                        params![parent_path, candidate],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .optional()?;
                if direct_match.is_some() {
                    return Ok(direct_match);
                }

                let mut stmt = conn.prepare(
                    "SELECT a.id, f.id, f.filename
                     FROM file_entries f
                     JOIN asset_files af ON af.file_id = f.id
                     JOIN assets a ON a.id = af.asset_id
                     WHERE f.parent_path = ?1
                       AND f.is_deleted = 0",
                )?;
                let rows = stmt.query_map(params![parent_path], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?;
                for row in rows {
                    let (asset_id, file_id, filename) = row?;
                    if let Some(score) = takeout_match_score(candidate, &filename) {
                        if best_match
                            .as_ref()
                            .map(|(current, _, _)| score > *current)
                            .unwrap_or(true)
                        {
                            best_match = Some((score, asset_id, file_id));
                        }
                    }
                }
            }

            Ok(best_match.map(|(_, asset_id, file_id)| (asset_id, file_id)))
        })
    }

    fn list_albums(&self) -> Result<Vec<AlbumSummary>, AppError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT al.id,
                        al.name,
                        al.source_path,
                        COUNT(DISTINCT aa.asset_id),
                        MIN(COALESCE(a.taken_at_utc, f.mtime_utc)),
                        MAX(COALESCE(a.taken_at_utc, f.mtime_utc))
                 FROM albums al
                 LEFT JOIN album_assets aa ON aa.album_id = al.id
                 LEFT JOIN assets a ON a.id = aa.asset_id AND a.is_deleted = 0
                 LEFT JOIN file_entries f ON f.id = a.primary_file_id
                 GROUP BY al.id, al.name, al.source_path
                 ORDER BY MIN(COALESCE(a.taken_at_utc, f.mtime_utc)) ASC, al.name COLLATE NOCASE",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(AlbumSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    source_path: row.get(2)?,
                    asset_count: row.get(3)?,
                    begin_taken_at_utc: row.get(4)?,
                    end_taken_at_utc: row.get(5)?,
                })
            })?;
            Ok(rows.filter_map(Result::ok).collect())
        })
    }

    fn list_assets_by_date(
        &self,
        request: AssetListRequest,
    ) -> Result<AssetListResponse, AppError> {
        paged_asset_query(
            self,
            "SELECT a.id, a.title, a.media_kind, a.taken_at_utc, a.duration_ms,
                    EXISTS(
                      SELECT 1 FROM asset_files af_live
                      WHERE af_live.asset_id = a.id AND af_live.role = 'live_photo_video'
                    ),
                    f.path, COALESCE(group_concat(DISTINCT al.name), '')
             FROM assets a
             JOIN file_entries f ON f.id = a.primary_file_id
             LEFT JOIN album_assets aa ON aa.asset_id = a.id
             LEFT JOIN albums al ON al.id = aa.album_id
             WHERE a.is_deleted = 0
               AND a.media_kind IN ('photo', 'video', 'live_photo')
               AND NOT EXISTS (
                 SELECT 1
                 FROM asset_files af_hidden
                 WHERE af_hidden.file_id = a.primary_file_id
                   AND af_hidden.role = 'live_photo_video'
               )",
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
                "SELECT a.id, a.title, a.media_kind, a.taken_at_utc, a.duration_ms,
                        EXISTS(
                          SELECT 1 FROM asset_files af_live
                          WHERE af_live.asset_id = a.id AND af_live.role = 'live_photo_video'
                        ),
                        f.path, COALESCE(group_concat(DISTINCT al.name), '')
                 FROM assets a
                 JOIN album_assets aa_filter ON aa_filter.asset_id = a.id AND aa_filter.album_id = {album_id}
                 JOIN file_entries f ON f.id = a.primary_file_id
                 LEFT JOIN album_assets aa ON aa.asset_id = a.id
                 LEFT JOIN albums al ON al.id = aa.album_id
                 WHERE a.is_deleted = 0
                   AND a.media_kind IN ('photo', 'video', 'live_photo')
                   AND NOT EXISTS (
                     SELECT 1
                     FROM asset_files af_hidden
                     WHERE af_hidden.file_id = a.primary_file_id
                       AND af_hidden.role = 'live_photo_video'
                   )"
            ),
            " ORDER BY COALESCE(a.taken_at_utc, f.mtime_utc) ASC, a.id ASC",
            request,
        )
    }

    fn search_assets(&self, request: AssetListRequest) -> Result<AssetListResponse, AppError> {
        let query = request
            .query
            .clone()
            .unwrap_or_default()
            .replace('\'', "''");
        paged_asset_query(
            self,
            &format!(
                "SELECT a.id, a.title, a.media_kind, a.taken_at_utc, a.duration_ms,
                    EXISTS(
                      SELECT 1 FROM asset_files af_live
                      WHERE af_live.asset_id = a.id AND af_live.role = 'live_photo_video'
                    ),
                    f.path, COALESCE(group_concat(DISTINCT al.name), '')
             FROM search_fts s
             JOIN assets a ON a.id = s.asset_id
             JOIN file_entries f ON f.id = a.primary_file_id
             LEFT JOIN album_assets aa ON aa.asset_id = a.id
             LEFT JOIN albums al ON al.id = aa.album_id
             WHERE a.is_deleted = 0
               AND a.media_kind IN ('photo', 'video', 'live_photo')
               AND NOT EXISTS (
                 SELECT 1
                 FROM asset_files af_hidden
                 WHERE af_hidden.file_id = a.primary_file_id
                   AND af_hidden.role = 'live_photo_video'
               )
               AND search_fts MATCH '{query}'"
            ),
            " ORDER BY rank",
            request,
        )
    }

    fn get_asset_detail(&self, asset_id: i64) -> Result<AssetDetail, AppError> {
        self.with_connection(|conn| {
            let detail = conn.query_row(
                "SELECT a.id, a.title, a.media_kind, a.display_type, a.taken_at_utc, f.file_size, a.width, a.height, a.duration_ms,
                        a.gps_lat, a.gps_lon, f.path, COALESCE(group_concat(DISTINCT al.name), ''), sm.google_photos_url
                 FROM assets a
                 JOIN file_entries f ON f.id = a.primary_file_id
                 LEFT JOIN sidecar_metadata sm ON sm.asset_id = a.id
                 LEFT JOIN album_assets aa ON aa.asset_id = a.id
                 LEFT JOIN albums al ON al.id = aa.album_id
                 WHERE a.id = ?1
                 GROUP BY a.id, a.title, a.media_kind, a.display_type, a.taken_at_utc, f.file_size, a.width, a.height, a.duration_ms,
                          a.gps_lat, a.gps_lon, f.path, sm.google_photos_url",
                params![asset_id],
                |row| {
                    Ok(AssetDetail {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        media_kind: row.get(2)?,
                        display_type: row.get(3)?,
                        taken_at_utc: row.get(4)?,
                        file_size: row.get(5)?,
                        width: row.get(6)?,
                        height: row.get(7)?,
                        duration_ms: row.get(8)?,
                        gps_lat: row.get(9)?,
                        gps_lon: row.get(10)?,
                        primary_path: row.get(11)?,
                        albums: split_csv(row.get::<_, String>(12)?),
                        live_photo_video_path: None,
                        google_photos_url: row.get(13)?,
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
                 WHERE scope NOT IN ('thumb_gen', 'batch_viewer_transcode')
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

    fn get_logs_by_scope(&self, scopes: &[&str], limit: u32) -> Result<Vec<LogEntry>, AppError> {
        self.with_connection(|conn| {
            let mut sql = String::from(
                "SELECT id, created_at, level, scope, message, asset_id
                 FROM app_logs",
            );
            if !scopes.is_empty() {
                let placeholders = vec!["?"; scopes.len()].join(", ");
                sql.push_str(&format!(" WHERE scope IN ({placeholders})"));
            }
            sql.push_str(" ORDER BY id DESC LIMIT ?");

            let mut params = scopes
                .iter()
                .map(|scope| scope.to_string())
                .collect::<Vec<_>>();
            params.push(limit.to_string());

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
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

    fn set_viewer_video_transcode_status(
        &self,
        asset_id: i64,
        status: &str,
        cache_path: Option<&str>,
    ) -> Result<(), AppError> {
        self.with_connection(|conn| {
            conn.execute(
                "INSERT INTO viewer_video_transcodes (asset_id, status, cache_path, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(asset_id) DO UPDATE SET
                   status = excluded.status,
                   cache_path = excluded.cache_path,
                   updated_at = excluded.updated_at",
                params![asset_id, status, cache_path, utc_now()],
            )?;
            Ok(())
        })
    }

    fn get_viewer_video_playback_statuses(
        &self,
        asset_ids: &[i64],
    ) -> Result<HashMap<i64, String>, AppError> {
        self.with_connection(|conn| {
            if asset_ids.is_empty() {
                return Ok(HashMap::new());
            }

            let placeholders = vec!["?"; asset_ids.len()].join(", ");
            let sql = format!(
                "SELECT asset_id, status
                 FROM viewer_video_transcodes
                 WHERE status IN ('ready', 'native', 'requires_transcode') AND asset_id IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(asset_ids.iter()), |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(rows.into_iter().collect())
        })
    }

    fn clear_viewer_video_transcode_statuses(&self) -> Result<(), AppError> {
        self.with_connection(|conn| {
            conn.execute("DELETE FROM viewer_video_transcodes", [])?;
            Ok(())
        })
    }

    fn clear_viewer_video_transcode_statuses_for_assets(
        &self,
        asset_ids: &[i64],
    ) -> Result<(), AppError> {
        if asset_ids.is_empty() {
            return Ok(());
        }
        self.with_connection(|conn| {
            let placeholders = vec!["?"; asset_ids.len()].join(", ");
            let sql =
                format!("DELETE FROM viewer_video_transcodes WHERE asset_id IN ({placeholders})");
            let params = asset_ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>();
            conn.execute(&sql, rusqlite::params_from_iter(params.iter()))?;
            Ok(())
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
            filters.push(media_kind_filter_sql(kind));
        }
        if let Some(start) = request.date_from.as_deref() {
            filters.push(format!("a.taken_at_utc >= '{}'", start.replace('\'', "''")));
        }
        if let Some(end) = request.date_to.as_deref() {
            filters.push(format!("a.taken_at_utc <= '{}'", end.replace('\'', "''")));
        }
        if let Some(query) = request.query.as_deref() {
            let escaped = query.replace('\'', "''");
            filters.push(format!(
                "(a.title LIKE '%{escaped}%' OR f.filename LIKE '%{escaped}%')"
            ));
        }
        if !filters.is_empty() {
            sql.push_str(" AND ");
            sql.push_str(&filters.join(" AND "));
        }
        sql.push_str(
            " GROUP BY a.id, a.title, a.media_kind, a.taken_at_utc, a.duration_ms, f.path",
        );
        let count_sql = format!("SELECT COUNT(*) FROM ({sql}) counted_assets");
        sql.push_str(order_sql);
        sql.push_str(&format!(" LIMIT {limit} OFFSET {offset}"));

        let total_count = conn.query_row(&count_sql, [], |row| row.get::<_, u32>(0))?;
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| map_asset_list_item(row))?;
        let items = rows.filter_map(Result::ok).collect::<Vec<_>>();
        let has_more = items.len() == limit as usize;
        Ok(AssetListResponse {
            items,
            next_cursor: has_more.then_some(offset + limit as u32),
            total_count,
        })
    })
}

fn media_kind_filter_sql(kind: &str) -> String {
    match kind {
        "photo" => "a.media_kind IN ('photo', 'live_photo')".to_string(),
        "live_photo" => "EXISTS (
            SELECT 1
            FROM asset_files af_live
            WHERE af_live.asset_id = a.id
              AND af_live.role = 'live_photo_video'
        )"
        .to_string(),
        "video" => "a.media_kind = 'video'".to_string(),
        other => format!("a.media_kind = '{}'", other.replace('\'', "''")),
    }
}

fn map_asset_list_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssetListItem> {
    Ok(AssetListItem {
        id: row.get(0)?,
        title: row.get(1)?,
        media_kind: row.get(2)?,
        taken_at_utc: row.get(3)?,
        duration_ms: row.get(4)?,
        has_live_photo: row.get::<_, bool>(5)?,
        primary_path: row.get(6)?,
        albums: split_csv(row.get::<_, String>(7)?),
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
