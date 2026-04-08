import { useEffect } from "react";

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
  }, []);

  async function handleRefreshIndex() {
    const roots = state.rootsInput
      .split(";")
      .map((item) => item.trim())
      .filter(Boolean);
    await logClient("ui.import", `refresh requested for ${roots.length} roots`);
    const progress = await api.refreshIndex({ roots });
    state.setImportStatus(progress);
    await refreshDebugSurfaces();
    await refreshAllAssets();
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

  return (
    <div className="app-shell">
      <Sidebar
        rootsInput={state.rootsInput}
        importStatus={state.importStatus}
        albums={state.albums}
        selectedAlbumId={state.selectedAlbumId}
        onRootsInputChange={state.setRootsInput}
        onRefresh={handleRefreshIndex}
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

      <ViewerModal asset={state.selectedAsset} onClose={() => state.setSelectedAsset(undefined)} />
    </div>
  );
}
