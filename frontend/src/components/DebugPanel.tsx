import type { CacheStats, DiagnosticEntry, LogEntry } from "../lib/types";

type DebugPanelProps = {
  diagnostics: DiagnosticEntry[];
  logs: LogEntry[];
  cacheStats?: CacheStats;
  collapsed?: boolean;
  thumbBatchRunning?: boolean;
  thumbBatchStopping?: boolean;
  videoBatchRunning?: boolean;
  videoBatchStopping?: boolean;
  onToggleCollapsed: () => void;
  onStartThumbBatch: () => void;
  onStopThumbBatch: () => void;
  onOpenThumbLog: () => void;
  onStartBatchTranscode: () => void;
  onStopBatchTranscode: () => void;
  onOpenBatchTranscode: () => void;
  onOpenDiagnostics: () => void;
  onOpenAppLogs: () => void;
  onClearThumbnails: () => void;
  onClearViewerRenders: () => void;
};

export function DebugPanel({
  diagnostics,
  logs,
  cacheStats,
  collapsed = false,
  thumbBatchRunning,
  thumbBatchStopping,
  videoBatchRunning,
  videoBatchStopping,
  onToggleCollapsed,
  onStartThumbBatch,
  onStopThumbBatch,
  onOpenThumbLog,
  onStartBatchTranscode,
  onStopBatchTranscode,
  onOpenBatchTranscode,
  onOpenDiagnostics,
  onOpenAppLogs,
  onClearThumbnails,
  onClearViewerRenders,
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
                  className={`${
                    thumbBatchRunning ? "button-danger" : "button-primary"
                  }${thumbBatchRunning ? " button-working" : ""}`}
                  onClick={thumbBatchRunning ? onStopThumbBatch : onStartThumbBatch}
                  disabled={thumbBatchStopping}
                >
                  {thumbBatchStopping
                    ? "Stopping thumb gen"
                    : thumbBatchRunning
                      ? "Stop thumb gen"
                      : "Start thumb gen"}
                </button>
                <button className="button-secondary" onClick={onOpenThumbLog}>
                  Thumb gen log
                </button>
                <button className="button-danger" onClick={onClearThumbnails}>
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
                transcoded viewer media: {cacheStats.viewer_render_items} items •{" "}
                {Math.round((cacheStats.viewer_render_bytes / 1024 / 1024) * 10) / 10} MB
              </div>
              <div className="button-row">
                <button
                  className={`${
                    videoBatchRunning ? "button-danger" : "button-primary"
                  }${videoBatchRunning ? " button-working" : ""}`}
                  onClick={videoBatchRunning ? onStopBatchTranscode : onStartBatchTranscode}
                  disabled={videoBatchStopping}
                >
                  {videoBatchStopping
                    ? "Stopping batch transcode"
                    : videoBatchRunning
                      ? "Stop batch transcode"
                      : "Start batch transcode"}
                </button>
                <button className="button-secondary" onClick={onOpenBatchTranscode}>
                  Batch transcode log
                </button>
                <button className="button-danger" onClick={onClearViewerRenders}>
                  Clear transcoded
                </button>
              </div>
            </div>
            <div className="debug-cache-summary">
              <div className="muted">ingress report: {diagnostics.length} warnings</div>
              <button className="button-secondary" onClick={onOpenDiagnostics}>
                Ingress report {diagnostics.length}
              </button>
            </div>
          </div>
        ) : null}
      </div>
      <div className="debug-section">
        <div className="debug-cache-summary">
          <div className="muted">app logs: {logs.length} entries</div>
          <button className="button-secondary" onClick={onOpenAppLogs}>
            App logs {logs.length}
          </button>
        </div>
      </div>
    </aside>
  );
}
