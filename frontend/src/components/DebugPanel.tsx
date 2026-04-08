import type { CacheStats, DiagnosticEntry, LogEntry } from "../lib/types";

type DebugPanelProps = {
  diagnostics: DiagnosticEntry[];
  logs: LogEntry[];
  cacheStats?: CacheStats;
};

export function DebugPanel({ diagnostics, logs, cacheStats }: DebugPanelProps) {
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
        <div className="eyebrow">Ingress diagnostics</div>
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
        <div className="eyebrow">Recent logs</div>
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
