use std::{collections::HashMap, path::Path};

use rusqlite::{OptionalExtension, params};

use crate::{
    import::sidecar::takeout_match_score,
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
    fn upsert_files(&self, import_id: i64, scans: &[&FileScanRecord])
    -> Result<Vec<i64>, AppError>;
    fn mark_files_seen(&self, import_id: i64, paths: &[&str]) -> Result<(), AppError>;
    fn soft_delete_missing_files(&self, import_id: i64, roots: &[String]) -> Result<u32, AppError>;
    fn reconcile_assets_after_file_deletions(&self) -> Result<(Vec<i64>, Vec<i64>), AppError>;
    fn upsert_album(&self, source_path: &str) -> Result<i64, AppError>;
    fn upsert_asset_for_file(
        &self,
        file_id: i64,
        scan: &FileScanRecord,
        sidecar: Option<&ParsedSidecar>,
        duration_ms: Option<i64>,
    ) -> Result<i64, AppError>;
    fn attach_asset_file(&self, asset_id: i64, file_id: i64, role: &str) -> Result<(), AppError>;
    fn attach_album_asset(&self, album_id: i64, asset_id: i64) -> Result<(), AppError>;
    fn set_sidecar_metadata(
        &self,
        asset_id: i64,
        sidecar_file_id: Option<i64>,
        sidecar: &ParsedSidecar,
    ) -> Result<(), AppError>;
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

    fn upsert_files(
        &self,
        import_id: i64,
        scans: &[&FileScanRecord],
    ) -> Result<Vec<i64>, AppError> {
        self.with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            let now = utc_now();
            let ids = {
                let mut upsert = tx.prepare(
                    "INSERT INTO file_entries
                     (path, parent_path, filename, extension, detected_format, mime_type, file_size,
                      mtime_utc, ctime_utc, quick_hash, json_kind, last_seen_import_id, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)
                     ON CONFLICT(path) DO UPDATE SET
                        parent_path = excluded.parent_path, filename = excluded.filename,
                        extension = excluded.extension, detected_format = excluded.detected_format,
                        mime_type = excluded.mime_type, file_size = excluded.file_size,
                        mtime_utc = excluded.mtime_utc, ctime_utc = excluded.ctime_utc,
                        quick_hash = excluded.quick_hash, json_kind = excluded.json_kind,
                        is_deleted = 0, last_seen_import_id = excluded.last_seen_import_id,
                        updated_at = excluded.updated_at
                     RETURNING id",
                )?;
                scans.iter().map(|scan| {
                    upsert.query_row(
                        params![
                            scan.path, scan.parent_path, scan.filename, scan.extension,
                            scan.detected_format, scan.mime_type, scan.file_size, scan.mtime_utc,
                            scan.ctime_utc, scan.quick_hash, scan.json_kind, import_id, now
                        ],
                        |row| row.get(0),
                    )
                }).collect::<Result<Vec<_>, _>>()?
            };
            tx.commit()?;
            Ok(ids)
        })
    }

    fn mark_files_seen(&self, import_id: i64, paths: &[&str]) -> Result<(), AppError> {
        if paths.is_empty() {
            return Ok(());
        }
        self.with_connection(|conn| {
            let paths = serde_json::to_string(paths)?;
            conn.execute(
                "UPDATE file_entries SET last_seen_import_id = ?1
                 WHERE path IN (SELECT value FROM json_each(?2))",
                params![import_id, paths],
            )?;
            Ok(())
        })
    }

    fn soft_delete_missing_files(&self, import_id: i64, roots: &[String]) -> Result<u32, AppError> {
        self.with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut count = 0;
            for root in roots {
                let root = crate::util::path::normalize_path(Path::new(root));
                let root = root.trim_end_matches('/');
                let prefix = format!("{root}/");
                count += tx.execute(
                    "UPDATE file_entries
                     SET is_deleted = 1, updated_at = ?1
                     WHERE last_seen_import_id != ?2 AND is_deleted = 0
                       AND substr(path, 1, length(?3)) = ?3",
                    params![utc_now(), import_id, prefix],
                )? as u32;
            }
            tx.commit()?;
            Ok(count)
        })
    }

    fn reconcile_assets_after_file_deletions(&self) -> Result<(Vec<i64>, Vec<i64>), AppError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT a.id, a.primary_file_id FROM assets a
                     JOIN file_entries f ON f.id = a.primary_file_id
                     WHERE a.is_deleted = 0 AND f.is_deleted = 1",
            )?;
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
        duration_ms: Option<i64>,
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
                     SET primary_file_id = ?2, media_kind = ?3, title = ?4, taken_at_utc = ?5,
                         duration_ms = ?6, is_deleted = 0, updated_at = ?7,
                         gps_lat = ?8, gps_lon = ?9, gps_alt = ?10
                     WHERE id = ?1",
                    params![
                        asset_id,
                        file_id,
                        scan.candidate_type,
                        title,
                        taken_at,
                        duration_ms,
                        utc_now(),
                        sidecar.and_then(|item| item.geo_lat),
                        sidecar.and_then(|item| item.geo_lon),
                        sidecar.and_then(|item| item.geo_alt)
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
            let mut stmt = conn.prepare(&format!(
                "SELECT al.id, al.name, al.source_path,
                        COALESCE(visible.asset_count, 0), visible.first_date, visible.last_date
                 FROM albums al
                 LEFT JOIN (
                     SELECT aa.album_id, COUNT(*) AS asset_count,
                            MIN(COALESCE(a.taken_at_utc, f.mtime_utc)) AS first_date,
                            MAX(COALESCE(a.taken_at_utc, f.mtime_utc)) AS last_date
                     FROM album_assets aa
                     JOIN assets a ON a.id = aa.asset_id
                     JOIN file_entries f ON f.id = a.primary_file_id
                     WHERE {VISIBLE_ASSET_SQL}
                     GROUP BY aa.album_id
                 ) visible ON visible.album_id = al.id
                 ORDER BY visible.first_date ASC, al.name COLLATE NOCASE",
            ))?;
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
        paged_asset_query(self, None, request)
    }

    fn list_assets_by_album(
        &self,
        album_id: i64,
        request: AssetListRequest,
    ) -> Result<AssetListResponse, AppError> {
        paged_asset_query(self, Some(album_id), request)
    }

    fn search_assets(&self, request: AssetListRequest) -> Result<AssetListResponse, AppError> {
        self.list_assets_by_date(request)
    }

    fn get_asset_detail(&self, asset_id: i64) -> Result<AssetDetail, AppError> {
        self.with_connection(|conn| {
            let detail = conn.query_row(
                &format!(
                    "SELECT a.id, a.title, a.media_kind, a.display_type, a.taken_at_utc,
                            f.file_size, a.width, a.height, a.duration_ms, a.gps_lat, a.gps_lon,
                            f.path, {ALBUM_NAMES_SQL}, sm.google_photos_url
                     FROM assets a
                     JOIN file_entries f ON f.id = a.primary_file_id
                     LEFT JOIN sidecar_metadata sm ON sm.asset_id = a.id
                     WHERE a.id = ?1",
                ),
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
                        albums: read_album_names(row, 12)?,
                        live_photo_video_path: None,
                        google_photos_url: row.get(13)?,
                    })
                },
            )?;

            let live_photo = live_photo_path(conn, asset_id)?;
            Ok(AssetDetail {
                live_photo_video_path: live_photo,
                ..detail
            })
        })
    }

    fn get_live_photo_pair(&self, asset_id: i64) -> Result<Option<String>, AppError> {
        self.with_connection(|conn| live_photo_path(conn, asset_id))
    }

    fn get_ingress_diagnostics(&self) -> Result<Vec<DiagnosticEntry>, AppError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, import_id, severity, diagnostic_type, related_path, message, created_at
                 FROM ingress_diagnostics
                 ORDER BY id DESC
                 LIMIT 10000",
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
    album_id: Option<i64>,
    request: AssetListRequest,
) -> Result<AssetListResponse, AppError> {
    use rusqlite::{params_from_iter, types::Value};

    db.with_connection(|conn| {
        let offset = request.cursor.unwrap_or_default();
        let limit = request.limit.unwrap_or(200);
        let mut from_sql = String::from("FROM assets a");
        let mut values = Vec::<Value>::new();
        if let Some(album_id) = album_id {
            from_sql.push_str(
                " JOIN album_assets aa_filter
                    ON aa_filter.asset_id = a.id AND aa_filter.album_id = ?",
            );
            values.push(album_id.into());
        }
        from_sql.push_str(&format!(
            " JOIN file_entries f ON f.id = a.primary_file_id WHERE {VISIBLE_ASSET_SQL}",
        ));
        match request.media_kind.as_deref() {
            Some("photo") => from_sql.push_str(" AND a.media_kind IN ('photo', 'live_photo')"),
            Some("live_photo") => from_sql.push_str(&format!(" AND {HAS_LIVE_PHOTO_SQL}")),
            Some(kind) => {
                from_sql.push_str(" AND a.media_kind = ?");
                values.push(Value::Text(kind.to_owned()));
            }
            None => {}
        }
        if let Some(start) = request.date_from {
            from_sql.push_str(" AND a.taken_at_utc >= ?");
            values.push(Value::Text(start));
        }
        if let Some(end) = request.date_to {
            from_sql.push_str(" AND a.taken_at_utc <= ?");
            values.push(Value::Text(end));
        }
        if let Some(query) = request.query.filter(|query| !query.is_empty()) {
            from_sql.push_str(
                " AND (instr(lower(a.title), lower(?)) > 0
                    OR instr(lower(f.filename), lower(?)) > 0)",
            );
            values.push(Value::Text(query.clone()));
            values.push(Value::Text(query));
        }

        let count_sql = format!("SELECT COUNT(*) {from_sql}");
        let total_count =
            conn.query_row(&count_sql, params_from_iter(&values), |row| row.get(0))?;
        let direction = if album_id.is_some() { "ASC" } else { "DESC" };
        let sql = format!(
            "SELECT a.id, a.title, a.media_kind, a.taken_at_utc, a.duration_ms,
                    {HAS_LIVE_PHOTO_SQL}, f.path, {ALBUM_NAMES_SQL}
             {from_sql}
             ORDER BY COALESCE(a.taken_at_utc, f.mtime_utc) {direction}, a.id {direction}
             LIMIT ? OFFSET ?"
        );
        values.extend([i64::from(limit).into(), i64::from(offset).into()]);
        let mut stmt = conn.prepare(&sql)?;
        let items = stmt
            .query_map(params_from_iter(&values), map_asset_list_item)?
            .collect::<Result<Vec<_>, _>>()?;
        let next_offset = offset.saturating_add(items.len() as u32);
        Ok(AssetListResponse {
            next_cursor: (!items.is_empty() && next_offset < total_count).then_some(next_offset),
            items,
            total_count,
        })
    })
}

const VISIBLE_ASSET_SQL: &str = "a.is_deleted = 0 AND f.is_deleted = 0
    AND a.media_kind IN ('photo', 'video', 'live_photo')
    AND NOT EXISTS (
        SELECT 1 FROM asset_files af
        JOIN assets still ON still.id = af.asset_id AND still.is_deleted = 0
        WHERE af.file_id = a.primary_file_id AND af.role = 'live_photo_video'
    )";

const ALBUM_NAMES_SQL: &str = "(SELECT json_group_array(DISTINCT al.name ORDER BY al.name)
    FROM album_assets aa
    JOIN albums al ON al.id = aa.album_id
    WHERE aa.asset_id = a.id)";

fn live_photo_path(conn: &rusqlite::Connection, asset_id: i64) -> Result<Option<String>, AppError> {
    Ok(conn
        .query_row(
            "SELECT f.path
         FROM asset_files af
         JOIN file_entries f ON f.id = af.file_id AND f.is_deleted = 0
         WHERE af.asset_id = ?1 AND af.role = 'live_photo_video'
         ORDER BY f.id LIMIT 1",
            [asset_id],
            |row| row.get(0),
        )
        .optional()?)
}

const HAS_LIVE_PHOTO_SQL: &str = "EXISTS (
    SELECT 1 FROM asset_files af_live
    JOIN file_entries motion ON motion.id = af_live.file_id AND motion.is_deleted = 0
    WHERE af_live.asset_id = a.id AND af_live.role = 'live_photo_video'
)";

fn map_asset_list_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssetListItem> {
    Ok(AssetListItem {
        id: row.get(0)?,
        title: row.get(1)?,
        media_kind: row.get(2)?,
        taken_at_utc: row.get(3)?,
        duration_ms: row.get(4)?,
        has_live_photo: row.get::<_, bool>(5)?,
        primary_path: row.get(6)?,
        albums: read_album_names(row, 7)?,
    })
}

fn read_album_names(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Vec<String>> {
    serde_json::from_str(&row.get::<_, String>(index)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|item| item.to_str())
        .unwrap_or(path)
        .to_string()
}

#[cfg(test)]
mod search_tests {
    use super::*;
    use crate::db::Database;

    fn search_fixture() -> Database {
        let db = Database::new(Path::new(":memory:")).unwrap();
        db.with_connection(|conn| {
            conn.execute_batch(
                "
                INSERT INTO file_entries
                    (id, path, parent_path, filename, file_size, mtime_utc,
                     last_seen_import_id, created_at, updated_at)
                VALUES
                    (1, '/trip/IMG-1234.jpg', '/trip', 'IMG-1234.jpg', 1,
                     '2024-04-01', 1, '', ''),
                    (2, '/trip/IMG-1235.jpg', '/trip', 'IMG-1235.jpg', 1,
                     '2024-04-02', 1, '', ''),
                    (3, '/other/IMG-1234.jpg', '/other', 'IMG-1234.jpg', 1,
                     '2024-04-03', 1, '', '');
                INSERT INTO assets
                    (id, primary_file_id, media_kind, display_type, title,
                     taken_at_utc, created_at, updated_at)
                VALUES
                    (1, 1, 'photo', 'original', 'Mom''s 50%_Crop', '2024-04-01', '', ''),
                    (2, 2, 'video', 'original', 'Mom''s 500xCrop', '2024-04-02', '', ''),
                    (3, 3, 'photo', 'original', 'Outside album', '2024-04-03', '', '');
                INSERT INTO albums (id, name, source_path, created_at, updated_at)
                VALUES (1, 'Trip', '/trip', '', '');
                INSERT INTO album_assets (album_id, asset_id, added_at)
                VALUES (1, 1, ''), (1, 2, '');
                ",
            )?;
            Ok(())
        })
        .unwrap();
        db
    }

    fn request(query: &str) -> AssetListRequest {
        AssetListRequest {
            cursor: None,
            limit: None,
            query: Some(query.into()),
            media_kind: None,
            date_from: None,
            date_to: None,
        }
    }

    #[test]
    fn album_counts_match_visible_assets_and_preserve_commas_in_names() {
        let db = search_fixture();
        db.with_connection(|conn| {
            conn.execute_batch(
                "UPDATE albums SET name = 'Prague, 2024' WHERE id = 1;
                 INSERT INTO asset_files (asset_id, file_id, role) VALUES (1, 2, 'live_photo_video');
                 UPDATE assets SET is_deleted = 1 WHERE id = 3;
                 INSERT INTO album_assets (album_id, asset_id, added_at) VALUES (1, 3, '');",
            )?;
            Ok(())
        }).unwrap();
        let album = db.list_assets_by_album(1, request("")).unwrap();
        assert_eq!(db.list_albums().unwrap()[0].asset_count, album.total_count);
        assert_eq!(album.items[0].albums, ["Prague, 2024"]);
        assert_eq!(db.get_asset_detail(1).unwrap().albums, ["Prague, 2024"]);
    }

    #[test]
    fn searches_filename_substrings_in_timeline_and_album() {
        let db = search_fixture();
        for query in ["img-1234.jpg", "34.JP"] {
            let timeline = db.search_assets(request(query)).unwrap();
            assert_eq!(timeline.total_count, 2);
            assert_eq!(
                timeline
                    .items
                    .iter()
                    .map(|item| item.id)
                    .collect::<Vec<_>>(),
                vec![3, 1]
            );

            let album = db.list_assets_by_album(1, request(query)).unwrap();
            assert_eq!(album.total_count, 1);
            assert_eq!(album.items[0].id, 1);
        }
    }

    #[test]
    fn searches_titles_literally_and_composes_with_filters() {
        let db = search_fixture();
        for query in ["mom's 50%_", "%", "_"] {
            let timeline = db.search_assets(request(query)).unwrap();
            assert_eq!(timeline.total_count, 1);
            assert_eq!(timeline.items[0].id, 1);

            let album = db.list_assets_by_album(1, request(query)).unwrap();
            assert_eq!(album.total_count, 1);
            assert_eq!(album.items[0].id, 1);
        }

        let mut filtered = request("img-123");
        filtered.media_kind = Some("photo".into());
        filtered.date_from = Some("2024-04-01".into());
        filtered.date_to = Some("2024-04-02".into());
        filtered.limit = Some(1);
        let timeline = db.search_assets(filtered).unwrap();
        assert_eq!(timeline.total_count, 1);
        assert_eq!(timeline.items[0].id, 1);
        assert_eq!(timeline.next_cursor, None);
    }
}
