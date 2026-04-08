import { useEffect } from "react";
import { confirm, open } from "@tauri-apps/plugin-dialog";

import { Sidebar } from "../components/Sidebar";
import { Toolbar } from "../components/Toolbar";
import { MediaGrid } from "../components/MediaGrid";
import { ViewerModal } from "../components/ViewerModal";
import { DebugPanel } from "../components/DebugPanel";
import { logClient } from "../lib/logger";
import { api } from "../lib/tauri";
import { useAppState } from "../state/appState";
import type { AssetListRequest } from "../lib/types";

export function App() {
  const state = useAppState();

  async function refreshAllAssets() {
    const request: AssetListRequest = {
      limit: 400,
      query: state.query || undefined,
      media_kind: state.mediaKind || undefined,
      date_from: state.dateFrom || undefined,
      date_to: state.dateTo || undefined,
    };

    const response =
      state.viewMode === "album" && state.selectedAlbumId
        ? await api.listAssetsByAlbum(state.selectedAlbumId, request)
        : state.query
          ? await api.searchAssets(request)
          : await api.listAssetsByDate(request);
    state.setAssets(response.items);
    await logClient("ui.refresh", `loaded ${response.items.length} assets in ${state.viewMode} mode`);
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
    await refreshAllAssets();
  }

  async function handleShowTimeline() {
    state.setSelectedAlbumId(undefined);
    state.setViewMode("timeline");
    await logClient("ui.timeline", "switched to timeline");
    await refreshAllAssets();
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

    await api.resetLocalDatabase();
    window.location.reload();
  }

  return (
    <div className="app-shell">
      <Sidebar
        rootsInput={state.rootsInput}
        importStatus={state.importStatus}
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
          dateFrom={state.dateFrom}
          dateTo={state.dateTo}
          onQueryChange={state.setQuery}
          onMediaKindChange={state.setMediaKind}
          onDateFromChange={state.setDateFrom}
          onDateToChange={state.setDateTo}
          onApply={refreshAllAssets}
        />
        <div className="grid-frame">
          <MediaGrid assets={state.assets} onSelect={handleSelectAsset} />
        </div>
      </main>

      <DebugPanel
        diagnostics={state.diagnostics}
        logs={state.logs}
        cacheStats={state.cacheStats}
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
