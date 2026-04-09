import { useEffect, useRef, useState } from "react";
import dayjs from "dayjs";
import { confirm, open } from "@tauri-apps/plugin-dialog";

import { Sidebar } from "../components/Sidebar";
import { Toolbar } from "../components/Toolbar";
import { MediaGrid } from "../components/MediaGrid";
import { ViewerModal } from "../components/ViewerModal";
import { DebugPanel } from "../components/DebugPanel";
import { logClient } from "../lib/logger";
import { isTauriRuntime } from "../lib/runtime";
import { api } from "../lib/tauri";
import { useAppState } from "../state/appState";
import type { AssetListRequest, LogEntry } from "../lib/types";

const ASSET_PAGE_SIZE = 200;

export function App() {
  const state = useAppState();
  const tauriRuntime = isTauriRuntime();
  const [timelineLabel, setTimelineLabel] = useState<string>();
  const [nextAssetCursor, setNextAssetCursor] = useState<number>();
  const [loadingMoreAssets, setLoadingMoreAssets] = useState(false);
  const [thumbnailResetKey, setThumbnailResetKey] = useState(0);
  const [thumbLogOpen, setThumbLogOpen] = useState(false);
  const [thumbGenerationLogs, setThumbGenerationLogs] = useState<LogEntry[]>([]);
  const didInitFilterEffect = useRef(false);
  const assetQueryGenerationRef = useRef(0);

  async function fetchAssetsPage(
    options: {
      viewMode?: "timeline" | "album";
      selectedAlbumId?: number;
      query?: string;
      mediaKind?: string;
    },
    cursor?: number,
  ) {
    const viewMode = options.viewMode ?? state.viewMode;
    const selectedAlbumId =
      "selectedAlbumId" in options ? options.selectedAlbumId : state.selectedAlbumId;
    const query = options.query ?? state.query;
    const mediaKind = options.mediaKind ?? state.mediaKind;
    const request: AssetListRequest = {
      cursor,
      limit: ASSET_PAGE_SIZE,
      query: query || undefined,
      media_kind: mediaKind || undefined,
    };

    const response =
      viewMode === "album" && selectedAlbumId
        ? await api.listAssetsByAlbum(selectedAlbumId, request)
        : query
          ? await api.searchAssets(request)
          : await api.listAssetsByDate(request);

    return {
      response,
      viewMode,
    };
  }

  async function refreshAllAssets(options?: {
    viewMode?: "timeline" | "album";
    selectedAlbumId?: number;
    query?: string;
    mediaKind?: string;
  }) {
    const generation = ++assetQueryGenerationRef.current;
    setLoadingMoreAssets(false);
    const { response, viewMode } = await fetchAssetsPage(options ?? {}, undefined);
    if (generation !== assetQueryGenerationRef.current) {
      return;
    }
    state.setAssets(response.items);
    setNextAssetCursor(response.next_cursor ?? undefined);
    setThumbnailResetKey((value) => value + 1);
    setTimelineLabel(formatTimelineLabel(response.items[0]?.taken_at_utc));
    await logClient(
      "ui.refresh",
      `loaded ${response.items.length} assets in ${viewMode} mode next_cursor=${response.next_cursor ?? "end"}`,
    );
  }

  async function loadMoreAssets() {
    if (loadingMoreAssets || nextAssetCursor == null) {
      return;
    }

    const generation = assetQueryGenerationRef.current;
    setLoadingMoreAssets(true);
    try {
      const { response, viewMode } = await fetchAssetsPage({}, nextAssetCursor);
      if (generation !== assetQueryGenerationRef.current) {
        return;
      }

      const currentAssets = useAppState.getState().assets;
      const seen = new Set(currentAssets.map((asset) => asset.id));
      const appendedItems = response.items.filter((asset) => !seen.has(asset.id));
      state.setAssets([...currentAssets, ...appendedItems]);
      setNextAssetCursor(response.next_cursor ?? undefined);
      await logClient(
        "ui.refresh",
        `appended ${appendedItems.length} assets in ${viewMode} mode total=${currentAssets.length + appendedItems.length} next_cursor=${response.next_cursor ?? "end"}`,
      );
    } finally {
      if (generation === assetQueryGenerationRef.current) {
        setLoadingMoreAssets(false);
      }
    }
  }

  async function refreshDebugSurfaces() {
    const [albums, diagnostics, logs, cacheStats, importStatus] = await Promise.all([
      api.listAlbums(),
      api.getDiagnostics(),
      api.getRecentLogs(),
      api.getCacheStats(),
      api.getImportStatus(),
    ]);
    state.setAlbums(albums);
    state.setDiagnostics(diagnostics);
    state.setLogs(logs);
    state.setCacheStats(cacheStats);
    state.setImportStatus(importStatus);
  }

  useEffect(() => {
    void logClient("ui.bootstrap", "frontend booted");
    void api.clearThumbGenerationLogs();
    void refreshDebugSurfaces();
    void refreshAllAssets();
    // Initial bootstrap is intentionally one-shot.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const timer = window.setInterval(() => {
      void api.getCacheStats().then((cacheStats) => {
        state.setCacheStats(cacheStats);
      });
    }, 1000);

    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!thumbLogOpen) {
      return;
    }

    let cancelled = false;
    async function refreshThumbLogs() {
      try {
        const entries = await api.getThumbGenerationLogs();
        if (!cancelled) {
          setThumbGenerationLogs(entries);
        }
      } catch (error) {
        console.error("failed to load thumb generation logs", error);
      }
    }

    void refreshThumbLogs();
    const timer = window.setInterval(() => {
      void refreshThumbLogs();
    }, 1000);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [thumbLogOpen]);

  useEffect(() => {
    if (!didInitFilterEffect.current) {
      didInitFilterEffect.current = true;
      return;
    }
    const timer = window.setTimeout(() => {
      void refreshAllAssets();
    }, 150);
    return () => window.clearTimeout(timer);
  }, [state.mediaKind, state.query]);

  async function handleRefreshIndex() {
    const roots = state.rootsInput
      .split(";")
      .map((item) => item.trim())
      .filter(Boolean);
    await logClient("ui.import", `refresh requested for ${roots.length} roots`);
    state.setImportStatus({
      import_id: 0,
      status: "running",
      phase: "starting",
      files_scanned: 0,
      processed_files: 0,
      total_files: 0,
      files_added: 0,
      files_updated: 0,
      files_deleted: 0,
      assets_added: 0,
      assets_updated: 0,
      assets_deleted: 0,
      worker_count: 0,
      message: "starting refresh",
    });

    const poll = window.setInterval(async () => {
      try {
        const status = await api.getImportStatus();
        if (status) {
          state.setImportStatus(status);
          if (status.status === "completed" || status.status === "failed") {
            window.clearInterval(poll);
            await refreshDebugSurfaces();
            await refreshAllAssets();
          }
        }
      } catch (error) {
        console.error("failed to poll import status", error);
      }
    }, 400);

    try {
      await api.startRefreshIndex({ roots });
    } finally {
      window.setTimeout(() => {
        void api.getImportStatus().then((status) => {
          if (status) {
            state.setImportStatus(status);
          }
        });
      }, 50);
    }
  }

  async function handleBrowseRoot() {
    if (!tauriRuntime) {
      const message =
        "Folder browsing is only available in the desktop Tauri app. In browser mode, open the desktop app with `npm run dev`, or type a path manually if you are only testing the UI.";
      console.info("browse_unavailable_browser_mode");
      window.alert(message);
      return;
    }

    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Choose Google Photos Takeout root",
      });

      if (!selected || Array.isArray(selected)) {
        await logClient("ui.import", "browse dialog cancelled");
        return;
      }

      state.setRootsInput(selected);
      await logClient("ui.import", `selected takeout root ${selected}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      await logClient("ui.import", `browse dialog failed: ${message}`, "error");
      window.alert(`Browse failed: ${message}`);
    }
  }

  async function handleSelectAlbum(albumId: number) {
    state.setSelectedAlbumId(albumId);
    state.setViewMode("album");
    await logClient("ui.album", `selected album ${albumId}`);
    await refreshAllAssets({
      viewMode: "album",
      selectedAlbumId: albumId,
    });
  }

  async function handleShowTimeline() {
    state.setSelectedAlbumId(undefined);
    state.setViewMode("timeline");
    await logClient("ui.timeline", "switched to timeline");
    await refreshAllAssets({
      viewMode: "timeline",
      selectedAlbumId: undefined,
    });
  }

  async function handleSelectAsset(assetId: number) {
    const detail = await api.getAssetDetail(assetId);
    state.setSelectedAsset(detail);
    await logClient("ui.viewer", `opened asset ${assetId}`);
  }

  async function handleStepAsset(direction: -1 | 1) {
    const currentId = state.selectedAsset?.id;
    if (!currentId) return;
    const currentIndex = state.assets.findIndex((asset) => asset.id === currentId);
    if (currentIndex < 0) return;
    const nextAsset = state.assets[currentIndex + direction];
    if (!nextAsset) return;
    await handleSelectAsset(nextAsset.id);
  }

  async function handleResetDatabase() {
    const accepted = await confirm(
      "This will permanently delete the local index, logs, diagnostics, and cached app data, then reload the app. Your original Takeout files will not be modified. Continue?",
      {
        title: "Clear local database?",
        kind: "warning",
        okLabel: "Clear Database",
        cancelLabel: "Cancel",
      },
    );

    if (!accepted) {
      await logClient("ui.reset", "local database reset cancelled");
      return;
    }

    state.setAssets([]);
    state.setSelectedAsset(undefined);
    state.setAlbums([]);
    state.setDiagnostics([]);
    state.setLogs([]);
    state.setCacheStats(undefined);
    state.setImportStatus(undefined);
    state.setSelectedAlbumId(undefined);
    state.setViewMode("timeline");
    setNextAssetCursor(undefined);
    setLoadingMoreAssets(false);

    await api.resetLocalDatabase();
    window.location.reload();
  }

  async function handleClearDiagnostics() {
    await api.clearDiagnostics();
    await refreshDebugSurfaces();
  }

  async function handleClearLogs() {
    await api.clearLogs();
    setThumbGenerationLogs([]);
    await refreshDebugSurfaces();
  }

  async function handleClearThumbnails() {
    await api.clearThumbnailCache();
    setThumbnailResetKey((value) => value + 1);
    if (thumbLogOpen) {
      setThumbGenerationLogs(await api.getThumbGenerationLogs());
    }
    const cacheStats = await api.getCacheStats();
    state.setCacheStats(cacheStats);
  }

  async function handleOpenThumbLog() {
    setThumbLogOpen(true);
    setThumbGenerationLogs(await api.getThumbGenerationLogs());
  }

  async function handleClearThumbLog() {
    await api.clearThumbGenerationLogs();
    setThumbGenerationLogs([]);
    await refreshDebugSurfaces();
  }

  async function handleCopyThumbLog() {
    const text = thumbGenerationLogs
      .map(
        (entry) =>
          `${formatLogTimestamp(entry.created_at)} [${entry.level}] asset=${entry.asset_id ?? "?"} ${entry.message}`,
      )
      .join("\n");
    await navigator.clipboard.writeText(text);
  }

  async function handleClearViewerRenders() {
    await api.clearViewerRenderCache();
    const cacheStats = await api.getCacheStats();
    state.setCacheStats(cacheStats);
  }

  const selectedAssetIndex = state.selectedAsset
    ? state.assets.findIndex((asset) => asset.id === state.selectedAsset?.id)
    : -1;

  return (
    <div className="app-shell">
      <Sidebar
        rootsInput={state.rootsInput}
        importStatus={state.importStatus}
        browseEnabled={tauriRuntime}
        albums={state.albums}
        selectedAlbumId={state.selectedAlbumId}
        onRootsInputChange={state.setRootsInput}
        onBrowseRoot={handleBrowseRoot}
        onRefresh={handleRefreshIndex}
        onResetDatabase={handleResetDatabase}
        onShowTimeline={handleShowTimeline}
        onSelectAlbum={handleSelectAlbum}
      />

      <main className="panel content-panel">
        <Toolbar
          query={state.query}
          mediaKind={state.mediaKind}
          timelineLabel={state.viewMode === "timeline" ? timelineLabel : undefined}
          onQueryChange={state.setQuery}
          onMediaKindChange={state.setMediaKind}
        />
        <div className="grid-frame">
          <MediaGrid
            assets={state.assets}
            onSelect={handleSelectAsset}
            thumbnailResetKey={thumbnailResetKey}
            hasMore={nextAssetCursor != null}
            isLoadingMore={loadingMoreAssets}
            onLoadMore={loadMoreAssets}
            onLeadingDateChange={(value) => {
              if (state.viewMode === "timeline") {
                setTimelineLabel(formatTimelineLabel(value));
              }
            }}
          />
        </div>
      </main>

      <DebugPanel
        diagnostics={state.diagnostics}
        logs={state.logs}
        cacheStats={state.cacheStats}
        onOpenThumbLog={() => void handleOpenThumbLog()}
        onClearThumbnails={handleClearThumbnails}
        onClearViewerRenders={handleClearViewerRenders}
        onClearDiagnostics={handleClearDiagnostics}
        onClearLogs={handleClearLogs}
      />

      {thumbLogOpen ? (
        <div className="viewer-backdrop" onClick={() => setThumbLogOpen(false)}>
          <div className="thumb-log-card" onClick={(event) => event.stopPropagation()}>
            <div className="viewer-toolbar">
              <div>
                <div className="title">Thumb Generation Log</div>
                <div className="muted">
                  start, success, unavailable, and fail events with timestamps
                </div>
              </div>
              <div className="button-row">
                <button className="button-secondary" onClick={() => void handleOpenThumbLog()}>
                  Refresh
                </button>
                <button className="button-secondary" onClick={() => void handleCopyThumbLog()}>
                  Copy
                </button>
                <button className="button-secondary" onClick={() => void handleClearThumbLog()}>
                  Clear
                </button>
                <button className="button-danger" onClick={() => setThumbLogOpen(false)}>
                  Close
                </button>
              </div>
            </div>
            <div className="thumb-log-list">
              {thumbGenerationLogs.length > 0 ? (
                thumbGenerationLogs.map((entry) => (
                  <div key={entry.id} className="thumb-log-line">
                    <span className="thumb-log-timestamp">{formatLogTimestamp(entry.created_at)}</span>
                    <span className="thumb-log-message">
                      [{entry.level}] asset={entry.asset_id ?? "?"} {entry.message}
                    </span>
                  </div>
                ))
              ) : (
                <div className="empty-state">No thumbnail generation events recorded yet.</div>
              )}
            </div>
          </div>
        </div>
      ) : null}

      <ViewerModal
        asset={state.selectedAsset}
        hasPrevious={selectedAssetIndex > 0}
        hasNext={selectedAssetIndex >= 0 && selectedAssetIndex < state.assets.length - 1}
        onPrevious={() => void handleStepAsset(-1)}
        onNext={() => void handleStepAsset(1)}
        onClose={() => state.setSelectedAsset(undefined)}
      />
    </div>
  );
}

function formatTimelineLabel(value?: string | null) {
  if (!value) return undefined;
  const parsed = dayjs(value);
  if (!parsed.isValid()) return undefined;
  return parsed.format("MMMM YYYY");
}

function formatLogTimestamp(value?: string | null) {
  if (!value) return "";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  const year = parsed.getFullYear();
  const month = String(parsed.getMonth() + 1).padStart(2, "0");
  const day = String(parsed.getDate()).padStart(2, "0");
  const hours = String(parsed.getHours()).padStart(2, "0");
  const minutes = String(parsed.getMinutes()).padStart(2, "0");
  const seconds = String(parsed.getSeconds()).padStart(2, "0");
  const millis = String(parsed.getMilliseconds()).padStart(3, "0");
  return `${year}-${month}-${day} ${hours}:${minutes}:${seconds}.${millis}`;
}
