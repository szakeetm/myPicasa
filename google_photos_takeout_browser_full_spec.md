# Google Photos Takeout Read-Only Desktop Browser — Implementation Spec

## Goal

Build a **read-only desktop app** that browses a Google Photos Takeout library without modifying or copying the source assets.

### Core constraints
- Source Takeout folders are assumed to remain present on disk.
- The app must **not modify** the Takeout files.
- The app must **not copy or import assets** into app-managed storage.
- The **only persistent app-owned storage** should be the database.
- Thumbnails/previews may be generated **in memory only** and must not be persisted as files.
- The app should support **100k+ photos/videos** with responsive search, scrolling, and browsing.

### Functional requirements
- Ingest from Google Photos Takeout.
- Parse and index:
  - image/video files
  - sidecar JSON metadata
  - live photo / motion photo sidecar video files
  - edited versions
  - original album/folder name from Takeout path
- Refresh the DB on new Takeouts:
  - add new files
  - update changed files
  - detect removed files
- UI views:
  - chronological view
  - album view
- Grid behavior:
  - thumbnails generated after scrolling stops for a short debounce
  - thumbnail generation uses multiple cores
  - thumbnails are evicted/flushed from memory when scrolled away
- Viewer behavior:
  - click opens item fit-to-screen
  - photos support zoom/pan
  - videos play
  - live photos optionally play associated motion clip

## Recommended Technology Stack

### Final recommendation
- **Backend:** Rust
- **Desktop shell:** Tauri
- **Frontend:** TypeScript + React or Svelte
- **Database:** SQLite
- **Image processing:** libvips
- **HEIC/HEIF decode:** libheif
- **Video playback / poster extraction:** libmpv or FFmpeg

### Why this stack
This app is performance-sensitive in:
- indexing
- hashing
- metadata extraction
- thumbnail scheduling
- RAM cache management
- media decode
- large-library querying

Rust is a strong fit for these backend concerns, while Tauri provides a thin desktop shell with a webview-based UI.

### Important architectural choice
Use a **webview UI**, but **not** a browser-only application.

That means:
- frontend handles rendering and interaction
- backend handles all file IO, indexing, decoding, thumbnail generation, search, and caching

Do **not** depend on browser-native support for HEIC or unusual video codecs.

## High-Level Architecture

### App layers
1. **Filesystem layer**
   - reads original Takeout folders
   - never modifies them

2. **Import/index layer**
   - scans files
   - parses sidecars
   - links related files
   - updates DB incrementally

3. **Database layer**
   - stores metadata, relationships, and search indexes

4. **Media service layer**
   - on-demand thumbnail generation
   - viewer image/video loading
   - RAM-only caching
   - worker scheduling

5. **UI layer**
   - virtualized timeline/album grid
   - search/filter controls
   - fullscreen viewer

## Data Model

### Important concept
Do not treat each physical file as the primary UI object.

Instead distinguish between:
- **physical file**
- **logical asset**
- relationships between files (edited version, live-photo video, sidecar JSON)

A single logical photo may correspond to multiple physical files in Takeout.

### Main entities
- `asset`: logical media item shown in UI
- `file_entry`: physical file on disk
- `album`: Takeout folder/album
- `sidecar_metadata`: parsed JSON metadata
- `asset_relationship`: links between original/edited/live-photo components

## SQLite Schema (Suggested)

Note: exact schema can vary, but preserve the concepts below.

### `file_entries`
Represents every relevant file discovered on disk.

Fields:
- `id INTEGER PRIMARY KEY`
- `path TEXT NOT NULL UNIQUE`
- `parent_path TEXT NOT NULL`
- `filename TEXT NOT NULL`
- `extension TEXT`
- `mime_type TEXT`
- `file_size INTEGER NOT NULL`
- `mtime_utc TEXT NOT NULL`
- `ctime_utc TEXT NULL`
- `quick_hash BLOB NULL`
- `full_hash BLOB NULL`
- `is_deleted INTEGER NOT NULL DEFAULT 0`
- `last_seen_import_id INTEGER NOT NULL`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`

Indexes:
- `idx_file_entries_filename`
- `idx_file_entries_mtime`
- `idx_file_entries_last_seen_import_id`
- `idx_file_entries_full_hash`

### `assets`
Represents logical UI-visible items.

Fields:
- `id INTEGER PRIMARY KEY`
- `primary_file_id INTEGER NOT NULL`
- `media_kind TEXT NOT NULL`  -- `photo`, `video`, `live_photo`
- `display_type TEXT NOT NULL` -- `original`, `edited`, etc.
- `taken_at_utc TEXT NULL`
- `taken_at_local TEXT NULL`
- `timezone_hint TEXT NULL`
- `width INTEGER NULL`
- `height INTEGER NULL`
- `duration_ms INTEGER NULL`
- `orientation INTEGER NULL`
- `gps_lat REAL NULL`
- `gps_lon REAL NULL`
- `gps_alt REAL NULL`
- `camera_make TEXT NULL`
- `camera_model TEXT NULL`
- `is_favorite INTEGER NOT NULL DEFAULT 0`
- `is_deleted INTEGER NOT NULL DEFAULT 0`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`

Indexes:
- `idx_assets_taken_at_utc`
- `idx_assets_media_kind`
- `idx_assets_primary_file_id`
- `idx_assets_deleted_taken_at`

### `asset_files`
Links logical assets to all related files.

Fields:
- `asset_id INTEGER NOT NULL`
- `file_id INTEGER NOT NULL`
- `role TEXT NOT NULL`

Roles may include:
- `primary`
- `original`
- `edited`
- `live_photo_video`
- `sidecar_json`
- `alternate`

Primary key:
- `(asset_id, file_id, role)`

Indexes:
- `idx_asset_files_file_id`
- `idx_asset_files_asset_role`

### `albums`
Represents Takeout folder/album groupings.

Fields:
- `id INTEGER PRIMARY KEY`
- `name TEXT NOT NULL`
- `source_path TEXT NOT NULL UNIQUE`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`

Indexes:
- `idx_albums_name`

### `album_assets`
Membership table.

Fields:
- `album_id INTEGER NOT NULL`
- `asset_id INTEGER NOT NULL`
- `position_hint INTEGER NULL`
- `added_at TEXT NOT NULL`

Primary key:
- `(album_id, asset_id)`

Indexes:
- `idx_album_assets_asset_id`
- `idx_album_assets_album_position`

### `sidecar_metadata`
Stores parsed JSON metadata.

Fields:
- `id INTEGER PRIMARY KEY`
- `asset_id INTEGER NOT NULL UNIQUE`
- `sidecar_file_id INTEGER NULL`
- `json_raw TEXT NULL`
- `photo_taken_time_utc TEXT NULL`
- `geo_lat REAL NULL`
- `geo_lon REAL NULL`
- `geo_alt REAL NULL`
- `people_json TEXT NULL`
- `google_photos_origin TEXT NULL`
- `import_version INTEGER NOT NULL DEFAULT 1`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`

### `asset_relationships`
Explicitly model relationships between assets.

Fields:
- `src_asset_id INTEGER NOT NULL`
- `dst_asset_id INTEGER NOT NULL`
- `relation_type TEXT NOT NULL`

Relation types may include:
- `edited_from`
- `motion_video_for`
- `duplicate_of`
- `variant_of`

Primary key:
- `(src_asset_id, dst_asset_id, relation_type)`

### `imports`
Tracks refresh/index runs.

Fields:
- `id INTEGER PRIMARY KEY`
- `source_root TEXT NOT NULL`
- `started_at TEXT NOT NULL`
- `finished_at TEXT NULL`
- `status TEXT NOT NULL`
- `files_scanned INTEGER NOT NULL DEFAULT 0`
- `files_added INTEGER NOT NULL DEFAULT 0`
- `files_updated INTEGER NOT NULL DEFAULT 0`
- `files_deleted INTEGER NOT NULL DEFAULT 0`
- `assets_added INTEGER NOT NULL DEFAULT 0`
- `assets_updated INTEGER NOT NULL DEFAULT 0`
- `assets_deleted INTEGER NOT NULL DEFAULT 0`
- `notes TEXT NULL`

### `search_fts`
FTS table for search.

Suggested indexed text:
- filename
- album name
- camera model
- year/month/day tokens
- optional location text if added later

### `ingress_diagnostics`
Stores ingest-only validation findings.

Fields:
- `id INTEGER PRIMARY KEY`
- `import_id INTEGER NOT NULL`
- `severity TEXT NOT NULL` -- `info`, `warning`, `error`
- `diagnostic_type TEXT NOT NULL`
- `asset_id INTEGER NULL`
- `file_id INTEGER NULL`
- `related_path TEXT NULL`
- `message TEXT NOT NULL`
- `details_json TEXT NULL`
- `created_at TEXT NOT NULL`

Suggested diagnostic types:
- `missing_referenced_asset`
- `orphan_asset`
- `orphan_sidecar_movie`
- `ambiguous_json_target`
- `unmerged_duplicate_candidate`
- `json_parse_failure`

Indexes:
- `idx_ingress_diag_import_id`
- `idx_ingress_diag_type`
- `idx_ingress_diag_asset_id`

## Asset Identification Strategy

Do **not** identify media by path alone.

Use a layered matching strategy:

1. **Fast path match**
   - path exists
   - size unchanged
   - mtime unchanged
   - treat as unchanged

2. **Candidate match**
   - same filename or similar filename
   - similar size
   - possible path change due to new Takeout layout

3. **Quick hash match**
   - hash first/last chunk(s)

4. **Full hash match**
   - used when needed for certainty

5. **Metadata correlation**
   - sidecar timestamp
   - dimensions
   - live photo pairing
   - edited/original naming patterns

### Recommended hashing behavior
To keep refresh fast:
- do **not** full-hash every file every refresh
- use `(path, size, mtime)` as the first-pass unchanged detector
- compute `quick_hash` for new or suspicious files
- compute `full_hash` only when required to confirm identity or detect content change

Possible `quick_hash` strategy:
- hash first 64 KiB + last 64 KiB + file size

Possible `full_hash` strategy:
- BLAKE3 preferred for speed

### File type detection rule
Do **not** infer image format from extension alone.

Google Photos historically had a “reduced quality” / compression flow that could rewrite images to JPEG while leaving misleading original-like filenames or extensions in Takeout-derived data. In practice, some files with `.png` or `.heic` extensions may actually contain JPEG bitstreams.

Therefore:
- detect file type from **magic bytes / container signature / decoder probe**, not extension alone
- store both `extension` and authoritative `detected_format`
- base decoder choice on `detected_format`
- treat extension only as a hint for classification and pairing heuristics

## Google Photos Takeout Parsing

Takeout exports are messy. Expect:
- sidecar JSON files
- duplicate exports across albums
- edited variants
- motion/live photo video companions
- filename collisions and numbering
- JSON/file naming mismatches

The importer must be tolerant.

### Importer responsibilities
- discover media files and sidecar JSON files
- infer associations between media and sidecars
- infer live photo still/video pairings
- infer original vs edited relationships when possible
- record album/folder origin from Takeout path
- preserve duplicate album membership without duplicating underlying assets unnecessarily

### Sidecar JSON
Extract and normalize at least:
- photo taken time
- upload/source time if present
- GPS coordinates
- people/labels only if cheap and useful
- any explicit original filename references if present

If sidecar parsing fails:
- keep asset/file indexed
- mark metadata parse status for diagnostics
- do not fail the entire import

### JSON parsing scope
Do not assume all JSON files are per-image sidecars.

The ingress layer should parse and classify **all JSON files** discovered under Takeout roots, including:
- image/video sidecar JSON
- album JSON
- manifest-like JSON
- any other Google Photos export JSON variant encountered

Unknown JSON variants should be:
- preserved as parsed/raw records where practical
- classified as best-effort `json_kind`
- included in diagnostics if they reference missing or ambiguous assets

## Refresh / Reindex Workflow

### Goals
Refresh should:
- be incremental
- be resumable if possible
- avoid unnecessary hashing/parsing
- detect adds/updates/deletes

### Refresh algorithm

#### Phase 1: scan filesystem
Walk configured Takeout roots and collect:
- path
- parent path
- filename
- extension
- size
- mtime
- candidate type (image/video/json/other)

Store into an in-memory scan set or temp table.

#### Phase 2: compare with DB
For each scanned file:
- if existing file row matches path + size + mtime -> unchanged
- else mark as new or changed

For new/changed files:
- detect actual file/container format from contents
- compute quick hash
- parse metadata headers if cheap
- parse sidecar JSON when applicable
- compute full hash only when needed
- update `file_entries`

#### Phase 3: rebuild relationships
For affected files/assets only:
- re-link sidecars
- re-link live photo still/video pairings
- re-link edited/original variants
- merge duplicate physical instances into a single logical asset when appropriate
- update album membership

#### Phase 4: detect deletions
Files present in DB but not seen in current scan:
- mark `is_deleted = 1`
- propagate to affected assets

Prefer soft-delete flags over hard deletes in v1 for simpler debugging.

#### Phase 5: ingress-only validation/test pass
Run a validation layer over the scanned/imported data.

This pass must parse/classify all JSON files and detect at least:
- JSON entries that reference missing assets
- orphan assets with no resolvable sidecar/relationship when one is expected
- orphan sidecar motion/live-photo movie files with no matched still asset
- sidecar JSON files with ambiguous or conflicting targets
- duplicate physical files that should be merged into one logical asset but are not

Persist findings into `ingress_diagnostics`.

#### Phase 6: finalize import
Update import statistics and commit.

### Transaction strategy
- Use batched transactions.
- Avoid one giant transaction for extremely large imports if it harms responsiveness.
- Consider:
  - one transaction per phase or chunk
  - WAL mode for SQLite

## Intelligent Duplicate Merge

### Problem case
Sometimes the same underlying photo exists in more than one Takeout location, for example:
- once under an album folder
- once under “All Photos” or a similar global folder

In some cases, the associated live photo / motion sidecar video exists in only **one** of those locations.

### Required behavior
The importer should intelligently merge such duplicates into a **single logical asset** when confidence is high.

This should be feasible because:
- albums reference `asset_id`
- albums do not require independent duplicated assets
- a single asset can have multiple `file_entry` instances and related files

### Merge heuristics
Prefer merging when a combination of these strongly agrees:
- same full hash
- same quick hash + same dimensions + same taken time
- same sidecar metadata identity
- same filename family / numbering pattern
- one duplicate has live-photo movie and the other does not

### Merge result
After merge:
- one logical `asset` remains primary
- both physical media paths may remain linked via `asset_files`
- album memberships from both locations point to the same `asset_id`
- the single discovered live-photo movie is attached to that merged asset
- duplicate relationships may be recorded in `asset_relationships`

### Non-goal
Do not present this as user-facing deduplication UI in v1. This is an ingest-time logical merge to improve browsing correctness.

## UI Requirements

### Main views
1. **Timeline view**
   - grouped chronologically
   - fast scrolling through date ranges
   - jump by year/month/day if useful

2. **Album view**
   - list of albums from Takeout folders
   - selecting album opens media grid

3. **Search/filter controls**
   - text search
   - media type filter
   - optional date range filter

4. **Viewer**
   - opens selected asset
   - fit-to-screen by default
   - zoom/pan for photos
   - playback for video
   - optional motion/live photo playback

### UI rendering constraints
- Must use a **virtualized list/grid**.
- Must not mount thousands of tile DOM nodes unnecessarily.
- Must fetch metadata in pages/windows.
- Must decouple scrolling from decode work.

## Thumbnail Strategy (No Persistent Thumbnail Files)

### Important constraint
The app must not persist generated thumbnails to disk.

Therefore:
- thumbnails are generated **on demand**
- only **RAM cache** is allowed
- moving away from a region may discard decoded thumbnails

### Design implications
This is acceptable, but the grid must be built around these facts:
- cold-cache scrolls will show placeholders briefly
- HEIC-heavy libraries can be slower than JPEG-heavy ones
- video poster extraction is more expensive than image thumbnails
- revisiting the same region after eviction will regenerate thumbnails

### Required behavior
- while scrolling is active:
  - avoid heavy decode/generation
  - prefer placeholders or already-cached thumbnails
- after scroll stops for a debounce interval (e.g. 120–200 ms):
  - schedule thumbnail jobs for visible items
- prioritize jobs by visibility and distance from viewport center
- cancel or deprioritize jobs for items that are no longer relevant
- evict thumbnails from RAM when far away or over memory budget

### Thumbnail pipeline
1. UI reports visible item IDs and viewport state.
2. Backend checks in-memory thumbnail cache.
3. If cached: return immediately.
4. If not cached and scrolling is idle long enough: enqueue job.
5. Worker generates thumbnail from source file.
6. Thumbnail bytes or pixel buffer returned to UI.
7. Store in RAM LRU cache.

### Multi-core behavior
Use a worker pool, but avoid overcommitting.

Recommended starting point:
- `min(physical_cores, 8)` workers for thumbnail generation
- make configurable

Do not saturate all logical threads by default, because storage contention and codec memory pressure can make performance worse.

### Image thumbnail generation
Preferred order:
1. embedded preview/EXIF thumbnail if available and suitable
2. direct decode + resize via libvips
3. decode respecting orientation metadata

### Video thumbnail generation
For grid tiles:
- generate a representative poster frame on demand
- do not persist it to disk
- cache in RAM only

## Scroll Performance Model

### Requirements
- Smooth perceived scrolling for large grids.
- No decode on UI thread.
- Placeholders acceptable during active scroll.

### Recommended behavior
- UI virtualization with overscan
- scrolling state machine:
  - `Scrolling`
  - `Settling`
  - `Idle`
- only schedule expensive work in `Idle`

### Suggested debounce timings
- scroll-stop debounce: `120–200 ms`
- cache eviction check: every `500–1000 ms` or on viewport changes

### Priority queue rules
Highest priority:
- items fully visible
- selected item
- nearest upcoming items in scroll direction

Lower priority:
- overscan items
- off-screen neighboring rows

Drop/cancel:
- items far from current viewport

## RAM Cache Design

Since no derived files may be written to disk, RAM cache quality matters.

### Suggested caches
1. **Thumbnail bitmap cache**
   - keyed by `(asset_id, target_size_bucket)`

2. **Metadata row cache**
   - keyed by `asset_id`

3. **Decoded viewer image cache** (small)
   - for selected/adjacent items only

### Cache policy
- LRU or segmented LRU
- memory budget configurable
- separate caps for thumbnails vs viewer assets

### Example initial budgets
Tune later, but possible defaults:
- thumbnail cache: `256–768 MB`
- metadata cache: `tens of MB`
- viewer cache: `128–512 MB`

Do not hardcode; make configurable or adaptive.

## Viewer Behavior

### Photos
On open:
- read source file directly
- display fit-to-screen preview first
- support zoom and pan
- load higher-resolution decode only when zoom level requires it

### Videos
On open:
- play directly from source file
- support pause/play/scrub/mute
- use hardware decoding when possible via playback library

### Live photos / motion photos
If a still image has a related motion clip:
- default viewer opens still image
- optional control to play motion clip
- optional autoplay mode if desired

### Performance guidance
Fullscreen viewing is expected to be fine reading directly from source on modern PCs, especially compared with the more demanding grid-thumbnail scenario.

## Search and Query Model

### Query types
Support at least:
- timeline pagination by date
- album listing
- album item listing
- text search
- media kind filtering
- date range filtering

### Indexing
Use standard SQLite indexes for:
- `taken_at_utc`
- `media_kind`
- album membership
- hash lookup
- deletion flags

Use FTS for:
- filename
- album name
- camera model (optional)
- other searchable metadata text

### Important query rule
The UI should never request all 100k items at once.
Use paged/windowed queries.

## Error Handling and Resilience

The importer must be resilient to:
- invalid JSON sidecars
- missing paired motion-video files
- files unsupported by decoders
- broken HEIC/video files
- duplicate/conflicting metadata
- partially removed Takeout directories

### Rules
- Index as much as possible.
- Record errors per file/asset/import.
- Do not abort the whole import for single-file failures.
- Surface diagnostics in logs and optionally a hidden debug view.

## Suggested Rust Module Layout

```text
src/
  main.rs
  app/
    commands.rs           # Tauri commands exposed to UI
    state.rs              # shared app state
  db/
    mod.rs
    schema.rs
    migrations.rs
    queries.rs
  import/
    mod.rs
    scanner.rs            # filesystem walking
    classifier.rs         # identify media/json/other
    matcher.rs            # file-to-asset linking
    sidecar.rs            # parse Google sidecar JSON
    json_classifier.rs    # classify album/media/other jsons
    validator.rs          # ingress-only validation checks
    dedupe_merge.rs       # logical duplicate merge
    live_photo.rs         # pair still/video
    refresher.rs          # incremental update orchestration
  media/
    mod.rs
    image.rs              # image decode helpers
    video.rs              # poster frame / playback helpers
    thumb.rs              # thumbnail generation pipeline
    viewer.rs             # fullscreen load helpers
  hash/
    mod.rs
    quick_hash.rs
    full_hash.rs
  cache/
    mod.rs
    lru.rs
    thumb_cache.rs
    metadata_cache.rs
  scheduler/
    mod.rs
    thumbnail_queue.rs
    priorities.rs
  search/
    mod.rs
    query_service.rs
  models/
    mod.rs
    asset.rs
    file_entry.rs
    album.rs
    relationships.rs
    diagnostic.rs
  util/
    time.rs
    path.rs
    errors.rs
```

## Suggested Frontend Structure

```text
frontend/src/
  app/
  components/
    MediaGrid.tsx
    MediaTile.tsx
    AlbumList.tsx
    TimelineView.tsx
    SearchBar.tsx
    ViewerModal.tsx
  hooks/
    useVisibleRange.ts
    useDebouncedScrollIdle.ts
    useAssetPage.ts
    useThumbnail.ts
  state/
    filters.ts
    selection.ts
    viewport.ts
  api/
    tauri.ts
  utils/
```

### Frontend responsibilities
- render virtualized UI
- detect visible item range
- debounce scroll stop
- request metadata windows and thumbnails
- display placeholders gracefully
- keep logic simple; avoid doing media decode in frontend JS

### Backend responsibilities
- all file IO
- hashing
- parsing
- query execution
- thumbnail generation
- cache and worker scheduling
- viewer source handling

## Tauri Command Surface (Suggested)

Examples only.

### Import / refresh
- `scan_takeout_roots(roots)`
- `refresh_index(roots)`
- `get_import_status()`
- `cancel_import()`
- `get_ingress_diagnostics(import_id, filters)`

### Queries
- `list_albums(offset, limit)`
- `list_assets_by_date(cursor, limit, filters)`
- `list_assets_by_album(album_id, cursor, limit)`
- `search_assets(query, filters, cursor, limit)`
- `get_asset_detail(asset_id)`

### Thumbnails
- `request_thumbnail(asset_id, size_bucket)`
- `request_thumbnails_batch(asset_ids, size_bucket)`
- `cancel_thumbnail_requests(request_ids)`
- `set_visible_range(context)`

### Viewer
- `open_viewer_asset(asset_id)`
- `load_viewer_frame(asset_id, target_size)`
- `open_video_stream(asset_id)`
- `get_live_photo_pair(asset_id)`

### Diagnostics
- `get_cache_stats()`
- `get_index_stats()`
- `get_last_import_summary()`

## Performance Targets

These are engineering targets, not guarantees.

### Initial targets
- App launches without preloading all media.
- Metadata queries for visible windows return quickly.
- Grid remains responsive while scrolling through 100k assets.
- Thumbnail generation is deferred until scroll idle.
- Clicking an item opens viewer quickly for most local SSD-backed libraries.

### What not to optimize prematurely
Do not build complex persistent derivative storage, transcoding pipelines, or distributed indexing in v1.

## v1 Scope Recommendation

### Include in v1
- single-user local desktop app
- one or more Takeout roots
- incremental refresh
- timeline view
- album view
- text search
- photo viewer with zoom
- video playback
- live photo pairing if feasible
- RAM-only thumbnail cache
- SQLite-only persistence
- ingress-only diagnostics for missing references/orphans/merge issues

### Defer from v1
- maps UI
- face/person grouping
- duplicate merge UI
- smart albums
- tag editing
- file export
- deletion actions
- persistent thumbnail files
- background sync service

## Implementation Notes / Opinions

1. **Do not rely on the webview/browser to decode source HEIC or video formats directly** for core functionality. Use native backend libraries.

2. **Do not store thumbnails in SQLite BLOBs** in this design. The constraint is DB-only persistence, but using the DB to persist rendered thumbnails would violate the spirit of lightweight indexing and bloat the DB.

3. **Prefer soft deletes** during refresh so that debugging importer behavior is easier.

4. **Use WAL mode** in SQLite.

5. **Use batched reads/writes** for import and querying.

6. **Keep the frontend dumb** about media processing.

7. **Treat grid thumbnails and fullscreen viewing separately**:
   - grid = aggressively scheduled, deferred, cancelable, RAM-cached
   - viewer = direct source load, progressively higher detail

8. **Never trust file extension alone for decoder selection or media-kind classification.** Use signature/header-based detection.

9. **Ingress validation is a first-class feature** for this data source because Google Takeout is structurally inconsistent.

## Deliverables for the Coding Agent

Please implement:

1. A Tauri desktop application with Rust backend.
2. SQLite schema and migrations based on the concepts above.
3. Incremental Takeout refresh/indexing pipeline.
4. Robust sidecar JSON parsing and asset linking.
5. Parsing/classification of all JSON files found during ingress.
6. Ingress-only validation layer for missing references, orphan assets, orphan sidecar movies, and merge diagnostics.
7. Intelligent duplicate merge so album/global duplicates resolve to one logical asset when confidence is high.
8. Timeline and album grid views with virtualization.
9. Debounced idle-triggered thumbnail generation.
10. In-memory thumbnail cache only; no persisted thumbnails.
11. Fullscreen viewer for photos and videos.
12. Search over indexed metadata.
13. Basic diagnostics/logging for import and decode failures.

## Acceptance Criteria

A build is acceptable when:
- user points app at Takeout folders
- app indexes them without modifying source files
- DB is the only persistent app-owned data
- app detects added/changed/removed files on refresh
- file type detection uses actual file contents, not extension alone
- timeline view and album view work
- scrolling remains responsive on large libraries due to virtualization
- thumbnails are generated only after scroll settles
- items scrolled far away no longer consume cache budget
- clicking a photo opens fit-to-screen and supports zoom
- clicking a video plays it
- live photo still/video relationship works when discoverable
- duplicate media present in multiple Takeout locations can merge into one logical asset when appropriate
- ingress diagnostics can report missing referenced assets, orphan assets, orphan sidecar movies, and ambiguous JSON references
- search returns relevant assets

## Summary Decision

This app should be built as a **native-core desktop app with a webview UI**, not as a browser-only web app.

**Recommended final architecture:**
- Rust backend
- Tauri shell
- TypeScript frontend
- SQLite for the only persistent app data
- direct source-file viewing
- on-demand thumbnail generation with **RAM-only caching**
- no copied assets and no persisted preview files
- signature-based format detection
- ingest-time diagnostics and intelligent duplicate merging
