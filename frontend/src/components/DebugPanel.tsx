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
  onClearThumbnails: () => void;
  onClearViewerRenders: () => void;
  onClearDiagnostics: () => void;
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
  onClearThumbnails,
  onClearViewerRenders,
  onClearDiagnostics,
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
            <<
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
          <div className="eyebrow">Ingress diagnostics</div>
          <button className="button-secondary" onClick={onClearDiagnostics}>
            Clear
          </button>
        </div>
        {diagnostics.slice(0, 8).map((diagnostic) => (
          <div key={diagnostic.id} className="diagnostic-item">
            <strong>{diagnostic.diagnostic_type}</strong>
            <div className="muted">
              {diagnostic.severity} • import {diagnostic.import_id}
            </div>
            <div>{diagnostic.message}</div>
            {diagnostic.related_path ? <div className="muted">{diagnostic.related_path}</div> : null}
          </div>
        ))}
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
            <div className="muted">{entry.created_at}</div>
          </div>
        ))}
      </div>
    </aside>
  );
}
