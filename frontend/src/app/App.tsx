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
import type { AssetListRequest } from "../lib/types";

export function App() {
  const state = useAppState();
  const tauriRuntime = isTauriRuntime();
  const [timelineLabel, setTimelineLabel] = useState<string>();
  const [thumbnailPreloadActive, setThumbnailPreloadActive] = useState(false);
  const [thumbnailPreloadRunId, setThumbnailPreloadRunId] = useState(0);
  const [thumbnailPreloadProgress, setThumbnailPreloadProgress] = useState<
    | {
        completed: number;
        total: number;
      }
    | undefined
  >();
  const didInitFilterEffect = useRef(false);

  async function refreshAllAssets(options?: {
    viewMode?: "timeline" | "album";
    selectedAlbumId?: number;
    query?: string;
    mediaKind?: string;
  }) {
    const viewMode = options?.viewMode ?? state.viewMode;
    const selectedAlbumId =
      options && "selectedAlbumId" in options ? options.selectedAlbumId : state.selectedAlbumId;
    const query = options?.query ?? state.query;
    const mediaKind = options?.mediaKind ?? state.mediaKind;
    const request: AssetListRequest = {
      limit: 400,
      query: query || undefined,
      media_kind: mediaKind || undefined,
    };

    const response =
      viewMode === "album" && selectedAlbumId
        ? await api.listAssetsByAlbum(selectedAlbumId, request)
        : query
          ? await api.searchAssets(request)
          : await api.listAssetsByDate(request);
    state.setAssets(response.items);
    setTimelineLabel(formatTimelineLabel(response.items[0]?.taken_at_utc));
    await logClient("ui.refresh", `loaded ${response.items.length} assets in ${viewMode} mode`);
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

    await api.resetLocalDatabase();
    window.location.reload();
  }

  async function handleClearDiagnostics() {
    await api.clearDiagnostics();
    await refreshDebugSurfaces();
  }

  async function handleClearLogs() {
    await api.clearLogs();
    await refreshDebugSurfaces();
  }

  async function handleClearThumbnails() {
    await api.clearThumbnailCache();
    const cacheStats = await api.getCacheStats();
    state.setCacheStats(cacheStats);
  }

  async function handleClearViewerRenders() {
    await api.clearViewerRenderCache();
    const cacheStats = await api.getCacheStats();
    state.setCacheStats(cacheStats);
  }

  function handleToggleThumbnailPreload() {
    if (thumbnailPreloadActive) {
      setThumbnailPreloadActive(false);
      setThumbnailPreloadProgress(undefined);
      return;
    }

    setThumbnailPreloadRunId((value) => value + 1);
    setThumbnailPreloadActive(true);
    setThumbnailPreloadProgress({
      completed: 0,
      total: state.assets.length,
    });
  }

  function handleThumbnailPreloadProgress(
    progress?: {
      completed: number;
      total: number;
    },
  ) {
    setThumbnailPreloadProgress((current) => {
      if (
        current?.completed === progress?.completed &&
        current?.total === progress?.total
      ) {
        return current;
      }
      return progress;
    });
    if (progress && progress.completed >= progress.total) {
      setThumbnailPreloadActive(false);
    }
  }

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
          thumbnailPreloadActive={thumbnailPreloadActive}
          thumbnailPreloadProgress={thumbnailPreloadProgress}
          onQueryChange={state.setQuery}
          onMediaKindChange={state.setMediaKind}
          onToggleThumbnailPreload={handleToggleThumbnailPreload}
        />
        <div className="grid-frame">
          <MediaGrid
            assets={state.assets}
            onSelect={handleSelectAsset}
            thumbnailPreload={{
              active: thumbnailPreloadActive,
              runId: thumbnailPreloadRunId,
            }}
            onThumbnailPreloadProgress={handleThumbnailPreloadProgress}
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
        onClearThumbnails={handleClearThumbnails}
        onClearViewerRenders={handleClearViewerRenders}
        onClearDiagnostics={handleClearDiagnostics}
        onClearLogs={handleClearLogs}
      />

      <ViewerModal
        asset={state.selectedAsset}
        hasPrevious={state.assets.findIndex((asset) => asset.id === state.selectedAsset?.id) > 0}
        hasNext={
          state.selectedAsset
            ? state.assets.findIndex((asset) => asset.id === state.selectedAsset?.id) < state.assets.length - 1
            : false
        }
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
