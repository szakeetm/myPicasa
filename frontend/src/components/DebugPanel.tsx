import type { CacheStats, DiagnosticEntry, LogEntry } from "../lib/types";

type DebugPanelProps = {
  diagnostics: DiagnosticEntry[];
  logs: LogEntry[];
  cacheStats?: CacheStats;
  collapsed?: boolean;
  thumbBatchRunning?: boolean;
  videoBatchRunning?: boolean;
  onToggleCollapsed: () => void;
  onOpenThumbLog: () => void;
  onOpenBatchTranscode: () => void;
  onOpenDiagnostics: () => void;
  onClearThumbnails: () => void;
  onClearViewerRenders: () => void;
  onClearLogs: () => void;
};

export function DebugPanel({
  diagnostics,
  logs,
  cacheStats,
  collapsed = false,
  thumbBatchRunning,
  videoBatchRunning,
  onToggleCollapsed,
  onOpenThumbLog,
  onOpenBatchTranscode,
  onOpenDiagnostics,
  onClearThumbnails,
  onClearViewerRenders,
  onClearLogs,
}: DebugPanelProps) {
  if (collapsed) {
    return (
      <aside className="panel debug-panel debug-panel-collapsed">
        <button
          className="debug-rail-toggle"
          type="button"
          onClick={onToggleCollapsed}
          aria-label="Expand diagnostics and logs panel"
          title="Expand diagnostics and logs panel"
        >
          <span className="debug-rail-chevron" aria-hidden="true">
            {"<<"}
          </span>
          <span className="debug-rail-label">Debug</span>
        </button>
      </aside>
    );
  }

  return (
    <aside className="panel debug-panel">
      <div className="header-block">
        <div className="debug-panel-heading">
          <div>
            <div className="eyebrow">Debug</div>
            <div className="title">Diagnostics and logs</div>
          </div>
          <button
            className="button-secondary debug-panel-toggle"
            type="button"
            onClick={onToggleCollapsed}
            aria-label="Collapse diagnostics and logs panel"
            title="Collapse diagnostics and logs panel"
          >
            Hide
          </button>
        </div>
        {cacheStats ? (
          <div className="debug-cache-group">
            <div className="debug-cache-summary">
              <div className="muted">
                thumbnails: {cacheStats.thumbnail_items} items •{" "}
                {Math.round((cacheStats.thumbnail_bytes / 1024 / 1024) * 10) / 10} MB
              </div>
              <div className="button-row">
                <button
                  className={`button-secondary${thumbBatchRunning ? " button-working" : ""}`}
                  onClick={onOpenThumbLog}
                >
                  Thumb gen log
                </button>
                <button
                  className={`button-secondary${videoBatchRunning ? " button-working" : ""}`}
                  onClick={onOpenBatchTranscode}
                >
                  Batch transcode
                </button>
                <button className="button-secondary" onClick={onClearThumbnails}>
                  Clear thumbnails
                </button>
              </div>
            </div>
            <div className="debug-cache-summary">
              <div className="muted">
                viewer previews: {cacheStats.preview_items} items •{" "}
                {Math.round((cacheStats.preview_bytes / 1024 / 1024) * 10) / 10} MB
              </div>
            </div>
            <div className="debug-cache-summary">
              <div className="muted">ingress diagnostics: {diagnostics.length} warnings</div>
              <button className="button-secondary" onClick={onOpenDiagnostics}>
                Diagnostics {diagnostics.length}
              </button>
            </div>
            <div className="debug-cache-summary">
              <div className="muted">
                rendered viewer media: {cacheStats.viewer_render_items} items •{" "}
                {Math.round((cacheStats.viewer_render_bytes / 1024 / 1024) * 10) / 10} MB
              </div>
              <button className="button-secondary" onClick={onClearViewerRenders}>
                Clear rendered
              </button>
            </div>
          </div>
        ) : null}
      </div>
      <div className="debug-section">
        <div className="button-row" style={{ justifyContent: "space-between", marginBottom: 10 }}>
          <div className="eyebrow">Recent logs</div>
          <button className="button-secondary" onClick={onClearLogs}>
            Clear logs
          </button>
        </div>
        {logs.slice(0, 20).map((entry) => (
          <div key={entry.id} className="log-item">
            <strong>
              {entry.level} • {entry.scope}
            </strong>
            <div>{entry.message}</div>
            <div className="muted">{formatLocalTimestamp(entry.created_at)}</div>
          </div>
        ))}
      </div>
    </aside>
  );
}

function formatLocalTimestamp(value?: string | null) {
  if (!value) return "";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  const year = parsed.getFullYear();
  const month = String(parsed.getMonth() + 1).padStart(2, "0");
  const day = String(parsed.getDate()).padStart(2, "0");
  const hours = String(parsed.getHours()).padStart(2, "0");
  const minutes = String(parsed.getMinutes()).padStart(2, "0");
  const seconds = String(parsed.getSeconds()).padStart(2, "0");
  return `${year}-${month}-${day} ${hours}:${minutes}:${seconds}`;
}
