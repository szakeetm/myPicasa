# myPicasa

Read-only desktop browser for Google Photos Takeout exports.

## Current v1

- Tauri desktop shell with a Rust backend
- React + TypeScript frontend
- SQLite index stored in the app data directory
- Incremental-style filesystem scan and index refresh
- Timeline and album browsing
- Search and basic filters
- RAM-only thumbnail cache
- Viewer for photos and videos
- Debug logging persisted in SQLite via `app_logs`
- Ingest diagnostics persisted in `ingress_diagnostics`

## Run

```bash
cd frontend
npm install
npm run tauri dev
```

## Notes

- Source Takeout files are read in place and never copied into app storage.
- Logs are stored in the SQLite database so debugging stays inside the app-owned DB.
- Thumbnail generation is currently image-first; video poster extraction is still a follow-up improvement.
