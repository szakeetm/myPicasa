import type { AlbumSummary, ImportProgress } from "../lib/types";
import dayjs from "dayjs";

type SidebarProps = {
  rootsInput: string;
  viewerPreviewSize: number;
  settingsCollapsed: boolean;
  importStatus?: ImportProgress | null;
  browseEnabled?: boolean;
  albums: AlbumSummary[];
  selectedAlbumId?: number;
  onRootsInputChange: (value: string) => void;
  onViewerPreviewSizeChange: (value: number) => void;
  onToggleSettingsCollapsed: () => void;
  onBrowseRoot: () => void;
  onRefresh: () => void;
  onResetDatabase: () => void;
  onShowTimeline: () => void;
  onSelectAlbum: (albumId: number) => void;
};

export function Sidebar({
  rootsInput,
  viewerPreviewSize,
  settingsCollapsed,
  importStatus,
  browseEnabled = true,
  albums,
  selectedAlbumId,
  onRootsInputChange,
  onViewerPreviewSizeChange,
  onToggleSettingsCollapsed,
  onBrowseRoot,
  onRefresh,
  onResetDatabase,
  onShowTimeline,
  onSelectAlbum,
}: SidebarProps) {
  const total = importStatus?.total_files ?? 0;
  const processed = importStatus?.processed_files ?? 0;
  const percent = total > 0 ? Math.min(100, Math.round((processed / total) * 100)) : 0;

  return (
    <aside className="panel sidebar">
      <div className="header-block">
        <div className="eyebrow">Google Photos Takeout</div>
        <div className="title">Read-only browser</div>
        <div className="muted">
          Indexes in SQLite and reads originals in place.
        </div>
        {importStatus ? (
          <div className="status-banner">
            {importStatus.status} • {importStatus.phase}
            <br />
            scanned {importStatus.files_scanned} files
            {total > 0 ? ` • ${processed}/${total} (${percent}%)` : ""}
            {importStatus.worker_count ? ` • ${importStatus.worker_count} workers` : ""}
            {importStatus.message ? ` • ${importStatus.message}` : ""}
          </div>
        ) : null}
      </div>

      <div className="controls controls-shell">
        <button
          className="section-toggle"
          type="button"
          onClick={onToggleSettingsCollapsed}
          aria-expanded={!settingsCollapsed}
          aria-controls="sidebar-settings-panel"
        >
          <span className="eyebrow section-toggle-label">Settings</span>
          <span className="section-toggle-icon" aria-hidden="true">
            {settingsCollapsed ? "▾" : "▴"}
          </span>
        </button>

        {!settingsCollapsed ? (
          <div id="sidebar-settings-panel" className="controls settings-panel">
            <div className="button-row">
              <input
                value={rootsInput}
                onChange={(event) => onRootsInputChange(event.target.value)}
                placeholder="/path/to/Takeout/Google Photos;/another/root"
              />
              <button
                className="button-secondary"
                onClick={onBrowseRoot}
                disabled={!browseEnabled}
                title={
                  browseEnabled
                    ? "Choose the Google Photos Takeout folder"
                    : "Folder browsing requires the desktop Tauri app"
                }
              >
                Browse
              </button>
            </div>
            <div className="muted">
              Pick the extracted Google Photos media root.
              <br />
              Usually this is the <strong>`Takeout/Google Photos`</strong> folder, or the specific
              subfolder that directly contains your album/media folders and sidecar JSON files.
            </div>
            {!browseEnabled ? (
              <div className="muted">
                Browser mode is for UI debugging only. Native folder browsing works in the desktop app.
              </div>
            ) : null}
            <div className="button-row">
              <button className="button-primary" onClick={onRefresh}>
                Refresh Index
              </button>
              <button className="button-danger" onClick={onResetDatabase}>
                Clear Local Database
              </button>
            </div>
            <div className="muted">
              Removes the local SQLite index, logs, albums, diagnostics, and cached app state.
              Source Takeout files are not touched.
            </div>
            <div className="setting-row">
              <label className="setting-label" htmlFor="viewer-preview-size">
                Viewer preview size
              </label>
              <select
                id="viewer-preview-size"
                value={String(viewerPreviewSize)}
                onChange={(event) => onViewerPreviewSizeChange(Number(event.target.value))}
              >
                <option value="1000">1000 px</option>
                <option value="1280">1280 px</option>
                <option value="1600">1600 px</option>
                <option value="2048">2048 px</option>
              </select>
            </div>
            <div className="muted">
              Controls the generated still-image size used for the viewer and grid preview warming.
            </div>
          </div>
        ) : null}
      </div>

      <div className="sidebar-section">
        <button className="button-secondary" onClick={onShowTimeline}>
          Timeline
        </button>
        <div className="eyebrow">Albums</div>
        {albums.length === 0 ? (
          <div className="muted">Refresh a Takeout root to populate albums.</div>
        ) : null}
        {albums.map((album) => (
          <button
            key={album.id}
            className={`album-item${selectedAlbumId === album.id ? " active" : ""}`}
            onClick={() => onSelectAlbum(album.id)}
          >
            <div>{album.name}</div>
            <div className="muted">
              {formatAlbumDateRange(album.begin_taken_at_utc, album.end_taken_at_utc)}
              <br />
              {album.asset_count} assets
              <br />
              {album.source_path}
            </div>
          </button>
        ))}
      </div>
    </aside>
  );
}

function formatAlbumDateRange(begin?: string | null, end?: string | null) {
  if (!begin && !end) {
    return "Unknown date range";
  }

  const beginDate = begin ? dayjs(begin) : undefined;
  const endDate = end ? dayjs(end) : undefined;

  if (beginDate?.isValid() && endDate?.isValid()) {
    if (beginDate.isSame(endDate, "day")) {
      return beginDate.format("YYYY-MM-DD");
    }
    if (beginDate.isSame(endDate, "month")) {
      return `${beginDate.format("YYYY-MM-DD")} to ${endDate.format("DD")}`;
    }
    if (beginDate.isSame(endDate, "year")) {
      return `${beginDate.format("YYYY-MM-DD")} to ${endDate.format("MM-DD")}`;
    }
    return `${beginDate.format("YYYY-MM-DD")} to ${endDate.format("YYYY-MM-DD")}`;
  }

  const fallback = beginDate?.isValid() ? beginDate : endDate;
  return fallback?.format("YYYY-MM-DD") ?? "Unknown date range";
}
