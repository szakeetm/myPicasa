import type { CacheStats, DiagnosticEntry, LogEntry } from "../lib/types";

type DebugPanelProps = {
  diagnostics: DiagnosticEntry[];
  logs: LogEntry[];
  cacheStats?: CacheStats;
  onClearDiagnostics: () => void;
  onClearLogs: () => void;
};

export function DebugPanel({
  diagnostics,
  logs,
  cacheStats,
  onClearDiagnostics,
  onClearLogs,
}: DebugPanelProps) {
  return (
    <aside className="panel debug-panel">
      <div className="header-block">
        <div className="eyebrow">Debug</div>
        <div className="title">Diagnostics and logs</div>
        {cacheStats ? (
          <div className="muted">
            cache: {cacheStats.thumbnail_items} thumbs •{" "}
            {Math.round(cacheStats.thumbnail_bytes / 1024 / 1024)} /{" "}
            {Math.round(cacheStats.thumbnail_budget_bytes / 1024 / 1024)} MB
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
