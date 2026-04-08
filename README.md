# myPicasa

Read-only desktop browser for Google Photos Takeout exports.

`myPicasa` is a Tauri desktop app with a Rust backend and React frontend. It indexes a Takeout library into SQLite, reads originals in place, generates thumbnails on demand, and gives you a fast timeline/album browser without rewriting your source files.

## Current Design

- Read-only by design: source Takeout files are never modified or copied into the library database.
- SQLite-backed index in the app data directory.
- Responsive desktop UI built with React, TypeScript, and Tauri.
- Timeline and album browsing.
- Search plus media/date filters.
- Photo viewer with zoom.
- Video playback and live photo playback from the same viewer modal.
- Sidecar JSON parsing for Google Photos metadata.
- Live photo companion movie detection and pairing.
- Ingest diagnostics and app logs persisted in SQLite.
- Thumbnail cache persisted on disk, with a memory working set for faster reuse.
- Thumbnail generation limited to tiles actually visible on screen.

## Architecture

### Frontend

- React 19 + TypeScript
- Zustand app state
- Tauri bridge for native commands
- CSS-based media grid and modal viewer

### Backend

- Rust + Tauri 2
- SQLite via `rusqlite`
- Filesystem scan with `walkdir`
- Parallel work with `rayon`
- Image decoding/resizing with `image`
- Google Photos sidecar parsing with `serde_json`

## External Runtime Dependencies

The app now relies on a couple of system tools for media handling:

- `ffmpeg`
  Used for video thumbnail extraction.
- `ffprobe`
  Used for video duration probing and thumbnail frame selection.
- `sips` on macOS
  Used as a fallback/helper for HEIC/HEIF and some image rendering paths.
- `qlmanage` on macOS
  Used for some thumbnail generation paths, especially HEIC/HEIF.

On macOS, `ffmpeg` and `ffprobe` are easiest to install with Homebrew:

```bash
brew install ffmpeg
```

## Project Dependencies

### Frontend packages

- `react`
- `react-dom`
- `zustand`
- `dayjs`
- `@tauri-apps/api`
- `@tauri-apps/plugin-dialog`

Note:
`@tanstack/react-virtual` is still present in `frontend/package.json`, but the grid currently uses a plain CSS layout rather than virtual rows.

### Rust crates

- `tauri`
- `tauri-plugin-dialog`
- `rusqlite`
- `serde`
- `serde_json`
- `chrono`
- `image`
- `walkdir`
- `rayon`
- `blake3`
- `tracing`
- `tracing-subscriber`
- `parking_lot`
- `mime_guess`
- `base64`
- `thiserror`

## Features

### Ingest

- Scans one or more Takeout roots.
- Parses Google Photos JSON sidecars.
- Uses sidecar timestamps where available.
- Ignores album-root `metadata.json` files as album metadata rather than media sidecars.
- Detects and pairs live photo companion movies with their still images.
- Merges duplicate assets when matching hashes indicate the same underlying media.
- Records diagnostics for unresolved or suspicious ingest cases.

### Browsing

- Album list in the sidebar.
- Timeline view for all indexed media.
- Search by title/filename.
- Media type filter.
- Date range filter.

### Thumbnail Cache

- Generated on demand.
- Stored on disk under the app data directory.
- Memory working set layered on top for faster repeated access.
- Cache stats exposed in the debug panel:
  count, total size, configured memory budget.
- Cache can be cleared from the UI with `Clear thumbnails`.

### Viewer

- Photos open in a zoomable viewer.
- Zoomed photos stay inside the viewer card and can be scrolled when enlarged.
- Videos play directly in the viewer.
- Live photo stills expose their companion motion clip in the viewer.

## Keyboard Shortcuts

Viewer shortcuts:

- `Left Arrow`
  Previous asset
- `Right Arrow`
  Next asset
- `Escape`
  Close viewer
- `+` or `=`
  Zoom in
- `-`
  Zoom out
- `0`
  Reset zoom to 100%

## Debug Panel

The right-hand debug panel currently exposes:

- Ingest diagnostics
- Recent logs
- Thumbnail cache stats
- Clear diagnostics
- Clear logs
- Clear thumbnails

## Run In Development

Install frontend dependencies first:

```bash
cd frontend
npm install
cd ..
```

Start the desktop app:

```bash
npm run dev
```

Useful additional commands:

```bash
npm run frontend:dev
npm run frontend:build
npm run build
```

## Build

Frontend only:

```bash
cd frontend
npm run build
```

Desktop app bundle:

```bash
npm run build
```

## Storage

App-managed data is stored in the Tauri app data directory and includes:

- `my_picasa.sqlite`
- persisted thumbnail cache files
- app-owned local state

The original Takeout media remains in its existing folders and is read in place.

## Current UX Notes

- Thumbnail work is intentionally limited to on-screen items.
- Live photos can show both a live-photo badge and a play affordance.
- Standalone movies show duration and a play affordance on the tile.
- Preview-generation debug logging is compiled in behind a flag and currently disabled by default.

## Known Follow-Ups

- The README documents the app as it behaves now, but the codebase still contains some earlier structures from the original prototype phase.
- If the loaded asset page size grows substantially, the grid may eventually want viewport windowing again, but it currently prioritizes tighter row packing and correct on-screen thumbnail requests.
