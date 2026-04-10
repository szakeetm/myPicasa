# myPicasa

Read-only desktop browser for Google Photos Takeout exports.

`myPicasa` is a Tauri desktop app with a Rust backend and React frontend. It indexes Google Photos Takeout media into SQLite, keeps the original files in place, generates derived media on demand, and gives you a fast local browser for large libraries, albums, live photos, and videos.

## What The App Does

- Reads Google Photos Takeout media in place without modifying originals.
- Indexes media metadata into a local SQLite database.
- Parses Google Photos JSON sidecars.
- Detects and pairs live photo motion clips with still images.
- Builds thumbnails, viewer previews, and rendered viewer-safe video files on demand.
- Browses media in timeline and album views.
- Searches assets by title and filename.
- Filters by media type:
  - all media
  - photos
  - live photos
  - videos
- Opens a viewer for photos, videos, and live photos.
- Tracks diagnostics, recent logs, thumbnail generation logs, and batch transcode logs.
- Lets you move generated cache storage to a custom folder, including external storage.
- Can export and import local app state for backup or migration.

## Core Principles

- Read-only for source media: original Takeout files are never rewritten.
- Local-first: database, caches, logs, and diagnostics live on your machine.
- Derived-data separation: originals stay where they are; generated outputs can be relocated.
- Practical large-library workflow: designed for browsing big Takeout archives and external drives.

## Features

## Browsing

- Timeline view across the full indexed library.
- Album list in the left sidebar.
- Album search by album name.
- Header date indicator based on the first visible asset in the current view.
- Total asset count for the current result set in the header.
- Hidden zero-asset albums in the sidebar.

## Search And Filters

- Full library search by title and filename.
- Media-type filter with:
  - `All media`
  - `Photos`
  - `Live photos`
  - `Videos`
- `Photos` includes live photos.
- `Live photos` shows only assets that have an attached live-photo motion file.

## Viewer

- Image viewer for photos.
- Video playback in the viewer.
- Live photo viewer with still image plus motion playback.
- Previous/next navigation between assets.
- Open the original file in Finder.
- Open the original file with the system default app.
- Open the original file with Quick Look on macOS.

## Thumbnail And Render Pipeline

- On-demand grid thumbnails.
- On-demand viewer previews for still images.
- Background rendered viewer-safe video transcodes.
- Thumbnail generation limited to visible or requested assets.
- Disk-backed caches with an in-memory working set.
- Cache stats visible in the debug panel.

## Cache Storage Relocation

- Configurable cache storage location for:
  - thumbnails
  - viewer previews
  - rendered viewer media
- Default cache location is the app-support directory.
- Database remains in the original app-support location.
- Optional copy of existing generated assets when changing cache storage.
- Progress modal during cache migration.
- Interrupt support during cache migration.

## Backup Export / Import

- Export local app state to a chosen folder.
- Export includes:
  - SQLite database
  - app settings
  - configured Takeout roots
  - thumbnail cache
  - preview cache
  - viewer render cache
- Import restores the same local app state from a backup folder.
- Import asks for the current Takeout root location(s).
- Import lets you choose a cache-storage location for the restored backup.
- Import can optionally run a refresh afterward. This is recommended.
- Backup export/import progress is shown in a modal.

## Debugging And Diagnostics

- Ingress diagnostics panel.
- Recent app logs.
- Thumbnail generation logs.
- Batch transcode logs.
- Cache stats for thumbnails, previews, and rendered viewer media.
- Clear diagnostics.
- Clear logs.
- Clear thumbnails/previews.
- Clear rendered viewer media.

## Ingest Pipeline

- Scans one or more Takeout roots.
- Parses sidecar JSON.
- Uses sidecar timestamps when available.
- Detects and attaches live-photo motion clips.
- Reconciles duplicates.
- Records diagnostics for suspicious or unresolved ingest cases.
- Refreshes the local index without altering originals.

## Storage Model

App-managed data includes:

- `my_picasa.sqlite`
- `settings.json`
- thumbnail cache files
- preview cache files
- rendered viewer media
- logs and diagnostics in SQLite

Original Takeout media remains in its existing filesystem location and is read directly.

## Backup Format

A backup export folder contains:

- `my_picasa.sqlite`
- `settings.json`
- `mypicasa-backup.json`
- `thumbnail-cache/`
- `preview-cache/`
- `viewer-cache/`

The backup manifest stores:

- export format version
- export timestamp
- app settings
- original indexed Takeout roots

## External Runtime Dependencies

The app depends on a few system tools for media handling:

- `ffmpeg`
  Used for video thumbnails and video transcoding.
- `ffprobe`
  Used for video probing and metadata extraction.
- `sips` on macOS
  Used for some image rendering paths and HEIC/HEIF handling.
- `qlmanage` on macOS
  Used in some macOS media preview paths.

On macOS, install `ffmpeg` and `ffprobe` with Homebrew:

```bash
brew install ffmpeg
```

## Tech Stack

## Frontend

- React 19
- TypeScript
- Zustand
- Day.js
- Tauri JS APIs

## Backend

- Rust
- Tauri 2
- SQLite via `rusqlite`
- `walkdir` for scanning
- `rayon` for parallel work
- `image` for image processing
- `serde_json` for sidecar parsing

## Project Layout

- [README.md](/Users/martonady/Repos/myPicasa/README.md)
  Main project overview.
- [frontend/](/Users/martonady/Repos/myPicasa/frontend)
  React/Tauri frontend.
- [src-tauri/](/Users/martonady/Repos/myPicasa/src-tauri)
  Rust backend and Tauri app shell.

## Development Setup

Install frontend dependencies:

```bash
cd frontend
npm install
cd ..
```

## Running The App

Start the Tauri desktop app in development:

```bash
npm run dev
```

Useful commands:

```bash
npm run frontend:dev
npm run frontend:build
npm run build
```

Frontend-only build:

```bash
cd frontend
npm run build
```

Desktop bundle build:

```bash
npm run build
```

## First-Time Use

1. Launch the app.
2. Choose the extracted `Takeout/Google Photos` folder, or another folder that directly contains the media and sidecar JSON files.
3. Click `Refresh Index`.
4. Browse the timeline or albums.
5. Optionally change cache storage to a larger external drive.
6. Optionally export a backup of the app state.

## Cache Storage Instructions

The app can store generated data in a custom cache folder.

Use this when:

- your library is large
- you want generated media on an external SSD
- you want to keep app-support storage small

Behavior:

- default location stays under the app-support directory
- changing cache storage can copy existing generated files
- if you choose not to copy, the app switches to an empty derived-data location
- the database stays in the original app-support directory

After moving cache storage:

- thumbnails
- previews
- rendered viewer media

will be read from the new location.

## Backup Instructions

## Export

Use `Export Backup` in the settings panel to create a portable copy of the local app state.

Recommended use cases:

- cloud backup
- migration to another machine
- moving between internal and external storage
- creating a restore point before major reindexing

## Import

Use `Import Backup` in the settings panel, then:

1. Select the exported backup folder.
2. Confirm or adjust the restored Takeout root location(s).
3. Optionally choose a cache storage location.
4. Choose whether to run a refresh after import.

Refresh after import is recommended because it:

- validates that the originals still exist
- updates metadata if the Takeout files changed
- refreshes album and file state against the current filesystem

## Reset Behavior

`Clear Local Database` removes local app state, including:

- indexed database contents
- thumbnails
- viewer previews
- rendered viewer media
- working files
- logs
- diagnostics

It does not modify the original Takeout media.

## Keyboard Shortcuts

Viewer shortcuts:

- `Left Arrow` for previous asset
- `Right Arrow` for next asset
- `Escape` to close the viewer
- `+` or `=` to zoom in
- `-` to zoom out
- `0` to reset zoom

## Notes

- Asset serving from custom cache folders is allowed from the default app-data area, the home directory, and mounted volumes as configured in [tauri.conf.json](/Users/martonady/Repos/myPicasa/src-tauri/tauri.conf.json).
- The grid currently favors responsive CSS layout and on-demand media generation over aggressive virtualization.
- The project still contains some earlier prototype structures, but the README reflects the app’s current behavior.
