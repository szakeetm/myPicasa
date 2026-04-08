import type { AlbumSummary, ImportProgress } from "../lib/types";

type SidebarProps = {
  rootsInput: string;
  importStatus?: ImportProgress | null;
  albums: AlbumSummary[];
  selectedAlbumId?: number;
  onRootsInputChange: (value: string) => void;
  onRefresh: () => void;
  onShowTimeline: () => void;
  onSelectAlbum: (albumId: number) => void;
};

export function Sidebar({
  rootsInput,
  importStatus,
  albums,
  selectedAlbumId,
  onRootsInputChange,
  onRefresh,
  onShowTimeline,
  onSelectAlbum,
}: SidebarProps) {
  return (
    <aside className="panel sidebar">
      <div className="header-block">
        <div className="eyebrow">Google Photos Takeout</div>
        <div className="title">Read-only browser</div>
        <div className="muted">
          Indexes in SQLite, reads originals in place, and keeps thumbnails RAM-only.
        </div>
        {importStatus ? (
          <div className="status-banner">
            {importStatus.status} • scanned {importStatus.files_scanned} files
            {importStatus.message ? ` • ${importStatus.message}` : ""}
          </div>
        ) : null}
      </div>

      <div className="controls">
        <input
          value={rootsInput}
          onChange={(event) => onRootsInputChange(event.target.value)}
          placeholder="/path/to/Takeout/Google Photos;/another/root"
        />
        <div className="button-row">
          <button className="button-primary" onClick={onRefresh}>
            Refresh Index
          </button>
          <button className="button-secondary" onClick={onShowTimeline}>
            Timeline
          </button>
        </div>
      </div>

      <div className="sidebar-section">
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
