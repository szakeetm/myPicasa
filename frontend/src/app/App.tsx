import { useEffect, useMemo, useRef, useState } from "react";
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
import type {
  AppBackupManifest,
  AssetListItem,
  AssetListRequest,
  BatchThumbnailGenerationStatus,
  BatchViewerTranscodeStatus,
  CacheStorageMigrationStatus,
  LogEntry,
  ViewerPlaybackSupport,
} from "../lib/types";

const ASSET_PAGE_SIZE = 200;

type GridPage = {
  startCursor: number;
  count: number;
  nextCursor: number | null;
  items: AssetListItem[] | null;
};

type GridEntry =
  | {
      kind: "asset";
      asset: AssetListItem;
    }
  | {
      kind: "placeholder";
      key: string;
      pageStartCursor: number;
      hydrationObserverKey: string | null;
    };

function sortGridPages(pages: GridPage[]) {
  return [...pages].sort((left, right) => left.startCursor - right.startCursor);
}

function loadedAssetsFromPages(pages: GridPage[]) {
  return sortGridPages(pages).flatMap((page) => page.items ?? []);
}

function gridEntriesFromPages(pages: GridPage[]): GridEntry[] {
  return sortGridPages(pages).flatMap((page) => {
    if (page.items) {
      return page.items.map((asset) => ({ kind: "asset" as const, asset }));
    }

    return Array.from({ length: page.count }, (_, index): GridEntry => ({
      kind: "placeholder" as const,
      key: `placeholder-${page.startCursor + index}`,
      pageStartCursor: page.startCursor,
      hydrationObserverKey:
        index === 0
          ? `start-${page.startCursor}`
          : index === page.count - 1
            ? `end-${page.startCursor}`
            : null,
    }));
  });
}

export function App() {
  const state = useAppState();
  const tauriRuntime = isTauriRuntime();
  const viewerPlaybackSupport = useMemo(() => getViewerPlaybackSupport(), []);
  const [gridPages, setGridPages] = useState<GridPage[]>([]);
  const [timelineLabel, setTimelineLabel] = useState<string>();
  const [viewAssetCount, setViewAssetCount] = useState(0);
  const [loadingPreviousAssets, setLoadingPreviousAssets] = useState(false);
  const [debugPanelCollapsed, setDebugPanelCollapsed] = useState(false);
  const [loadingMoreAssets, setLoadingMoreAssets] = useState(false);
  const [thumbnailResetKey, setThumbnailResetKey] = useState(0);
  const [viewerPreviewReadyAssetIds, setViewerPreviewReadyAssetIds] = useState<number[]>([]);
  const [thumbLogOpen, setThumbLogOpen] = useState(false);
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false);
  const [appLogsOpen, setAppLogsOpen] = useState(false);
  const [thumbGenerationLogs, setThumbGenerationLogs] = useState<LogEntry[]>([]);
  const [batchThumbnailStatus, setBatchThumbnailStatus] = useState<BatchThumbnailGenerationStatus>();
  const [batchTranscodeOpen, setBatchTranscodeOpen] = useState(false);
  const [batchTranscodeStatus, setBatchTranscodeStatus] = useState<BatchViewerTranscodeStatus>();
  const [batchTranscodeLogs, setBatchTranscodeLogs] = useState<LogEntry[]>([]);
  const [logAssetPaths, setLogAssetPaths] = useState<Record<number, string>>({});
  const [cacheStorageDir, setCacheStorageDir] = useState("");
  const [savedCacheStorageDir, setSavedCacheStorageDir] = useState("");
  const [cacheStorageChangePending, setCacheStorageChangePending] = useState<string | null>(null);
  const [cacheStorageMigrationStatus, setCacheStorageMigrationStatus] = useState<CacheStorageMigrationStatus>();
  const [cacheStorageMigrationModalOpen, setCacheStorageMigrationModalOpen] = useState(false);
  const [importBackupDir, setImportBackupDir] = useState<string>();
  const [importBackupManifest, setImportBackupManifest] = useState<AppBackupManifest>();
  const [importBackupRootsInput, setImportBackupRootsInput] = useState("");
  const [importBackupCacheStorageDir, setImportBackupCacheStorageDir] = useState("");
  const [importBackupShouldRefresh, setImportBackupShouldRefresh] = useState(true);
  const [backupTransferWorking, setBackupTransferWorking] = useState(false);
  const [backupTransferMode, setBackupTransferMode] = useState<"export" | "import">("export");
  const [backupTransferMessage, setBackupTransferMessage] = useState("");
  const didInitFilterEffect = useRef(false);
  const didInitViewerPreviewSizeEffect = useRef(false);
  const didInitRootsSettingsEffect = useRef(false);
  const cacheStorageMigrationWasRunningRef = useRef(false);
  const assetQueryGenerationRef = useRef(0);
  const gridPagesRef = useRef<GridPage[]>([]);
  const hydratingPageStartsRef = useRef(new Set<number>());

  const loadedGridPages = useMemo(
    () => sortGridPages(gridPages).filter((page) => page.items !== null),
    [gridPages],
  );
  const hasPreviousAssetPage = loadedGridPages.length > 0 && loadedGridPages[0].startCursor > 0;
  const hasNextAssetPage =
    loadedGridPages.length > 0 && loadedGridPages[loadedGridPages.length - 1].nextCursor != null;
  const gridEntries = useMemo(() => gridEntriesFromPages(gridPages), [gridPages]);
  const refreshRunning = state.importStatus?.status === "running";

  function commitGridPages(nextPages: GridPage[]) {
    const sortedPages = sortGridPages(nextPages);
    gridPagesRef.current = sortedPages;
    setGridPages(sortedPages);
    state.setAssets(loadedAssetsFromPages(sortedPages));
  }

  function commitHydratedPage(pageStart: number, items: AssetListItem[], nextCursor: number | null) {
    const pageMap = new Map(gridPagesRef.current.map((page) => [page.startCursor, page]));
    pageMap.set(pageStart, {
      startCursor: pageStart,
      count: items.length,
      nextCursor,
      items,
    });

    const hydratedPages = sortGridPages(Array.from(pageMap.values())).filter((page) => page.items !== null);
    const keepStarts = new Set<number>([pageStart]);
    const nearestNeighbor = hydratedPages
      .filter((page) => page.startCursor !== pageStart)
      .sort(
        (left, right) =>
          Math.abs(left.startCursor - pageStart) - Math.abs(right.startCursor - pageStart),
      )[0];

    if (nearestNeighbor) {
      keepStarts.add(nearestNeighbor.startCursor);
    }

    for (const [startCursor, page] of pageMap.entries()) {
      if (page.items !== null && !keepStarts.has(startCursor)) {
        pageMap.set(startCursor, {
          ...page,
          count: page.items.length,
          items: null,
        });
      }
    }

    commitGridPages(Array.from(pageMap.values()));
  }

  async function confirmDestructiveAction(title: string, message: string, okLabel: string) {
    if (tauriRuntime) {
      return confirm(message, {
        title,
        kind: "warning",
        okLabel,
        cancelLabel: "Cancel",
      });
    }
    return window.confirm(message);
  }

  function currentIndexedRoots() {
    return state.rootsInput
      .split(";")
      .map((item) => item.trim())
      .filter(Boolean);
  }

  function buildAppSettings(overrides?: Partial<{
    viewerPreviewSize: number;
    cacheStorageDir: string | null;
    indexedRoots: string[];
  }>) {
    return {
      viewer_preview_size: overrides?.viewerPreviewSize ?? state.viewerPreviewSize,
      cache_storage_dir:
        overrides?.cacheStorageDir !== undefined ? overrides.cacheStorageDir : cacheStorageDir || null,
      indexed_roots: overrides?.indexedRoots ?? currentIndexedRoots(),
    };
  }

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
    setLoadingPreviousAssets(false);
    setLoadingMoreAssets(false);
    const { response, viewMode } = await fetchAssetsPage(options ?? {}, undefined);
    if (generation !== assetQueryGenerationRef.current) {
      return;
    }
    commitGridPages([
      {
        startCursor: 0,
        count: response.items.length,
        nextCursor: response.next_cursor ?? null,
        items: response.items,
      },
    ]);
    setViewAssetCount(response.total_count);
    setThumbnailResetKey((value) => value + 1);
    setViewerPreviewReadyAssetIds([]);
    setTimelineLabel(formatTimelineLabel(response.items[0]?.taken_at_utc));
    await logClient(
      "ui.refresh",
      `loaded ${response.items.length} assets in ${viewMode} mode start_cursor=0 next_cursor=${response.next_cursor ?? "end"}`,
    );
  }

  async function loadMoreAssets() {
    if (loadingMoreAssets || !hasNextAssetPage) {
      return;
    }

    const generation = assetQueryGenerationRef.current;
    setLoadingMoreAssets(true);
    try {
      const loadedPages = sortGridPages(gridPagesRef.current).filter((page) => page.items !== null);
      const lastLoadedPage = loadedPages[loadedPages.length - 1];
      const nextCursor = lastLoadedPage?.nextCursor;
      if (nextCursor == null) {
        return;
      }
      const { response, viewMode } = await fetchAssetsPage({}, nextCursor);
      if (generation !== assetQueryGenerationRef.current) {
        return;
      }

      setViewAssetCount(response.total_count);
      const pageStart = nextCursor;
      commitHydratedPage(pageStart, response.items, response.next_cursor ?? null);
      await logClient(
        "ui.refresh",
        `loaded page start=${pageStart} count=${response.items.length} in ${viewMode} mode next_cursor=${response.next_cursor ?? "end"}`,
      );
    } finally {
      if (generation === assetQueryGenerationRef.current) {
        setLoadingMoreAssets(false);
      }
    }
  }

  async function loadPreviousAssets() {
    if (loadingPreviousAssets || !hasPreviousAssetPage) {
      return;
    }

    const generation = assetQueryGenerationRef.current;
    setLoadingPreviousAssets(true);
    try {
      const loadedPages = sortGridPages(gridPagesRef.current).filter((page) => page.items !== null);
      const firstLoadedPage = loadedPages[0];
      if (!firstLoadedPage) {
        return;
      }
      const previousCursor = Math.max(0, firstLoadedPage.startCursor - ASSET_PAGE_SIZE);
      const { response, viewMode } = await fetchAssetsPage({}, previousCursor);
      if (generation !== assetQueryGenerationRef.current) {
        return;
      }

      setViewAssetCount(response.total_count);
      commitHydratedPage(previousCursor, response.items, response.next_cursor ?? null);
      await logClient(
        "ui.refresh",
        `loaded page start=${previousCursor} count=${response.items.length} in ${viewMode} mode direction=up`,
      );
    } finally {
      if (generation === assetQueryGenerationRef.current) {
        setLoadingPreviousAssets(false);
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
    if (tauriRuntime) {
      void api.getAppSettings().then((settings) => {
        state.setViewerPreviewSize(settings.viewer_preview_size);
        setCacheStorageDir(settings.cache_storage_dir ?? "");
        setSavedCacheStorageDir(settings.cache_storage_dir ?? "");
        state.setRootsInput(settings.indexed_roots.join(";"));
      });
      void api.getCacheStorageMigrationStatus().then((status) => {
        setCacheStorageMigrationStatus(status);
        cacheStorageMigrationWasRunningRef.current = status.running;
      });
    }
    void refreshDebugSurfaces();
    void refreshAllAssets();
    void Promise.all([
      api.getBatchThumbnailGenerationStatus(),
      api.getBatchViewerTranscodeStatus(),
    ]).then(([thumbnailStatus, transcodeStatus]) => {
      setBatchThumbnailStatus(thumbnailStatus);
      setBatchTranscodeStatus(transcodeStatus);
    });
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
    const shouldTrackThumbnailBatch = thumbLogOpen || batchThumbnailStatus?.status === "running";
    if (!shouldTrackThumbnailBatch) {
      return;
    }

    let cancelled = false;
    async function refreshThumbLogs() {
      try {
        const status = await api.getBatchThumbnailGenerationStatus();
        if (!cancelled) {
          setBatchThumbnailStatus(status);
        }
        if (!thumbLogOpen) {
          return;
        }
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
  }, [batchThumbnailStatus?.status, thumbLogOpen]);

  useEffect(() => {
    const shouldTrackBatchTranscode = batchTranscodeOpen || batchTranscodeStatus?.status === "running";
    if (!shouldTrackBatchTranscode) {
      return;
    }

    let cancelled = false;
    async function refreshBatchStatus() {
      try {
        const status = await api.getBatchViewerTranscodeStatus();
        if (!cancelled) {
          setBatchTranscodeStatus(status);
        }
        if (!batchTranscodeOpen) {
          return;
        }
        const logs = await api.getBatchViewerTranscodeLogs();
        if (!cancelled) {
          setBatchTranscodeLogs(logs);
        }
      } catch (error) {
        console.error("failed to load batch transcode status", error);
      }
    }

    void refreshBatchStatus();
    const timer = window.setInterval(() => {
      void refreshBatchStatus();
    }, 1000);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [batchTranscodeOpen, batchTranscodeStatus?.status]);

  useEffect(() => {
    if (!tauriRuntime) {
      return;
    }
    const shouldPoll =
      cacheStorageMigrationStatus?.running ||
      cacheStorageMigrationStatus?.status === "completed" ||
      cacheStorageMigrationStatus?.status === "failed" ||
      cacheStorageMigrationStatus?.status === "cancelled";
    if (!shouldPoll) {
      return;
    }

    let cancelled = false;
    async function refreshMigrationStatus() {
      try {
        const status = await api.getCacheStorageMigrationStatus();
        if (cancelled) {
          return;
        }
        setCacheStorageMigrationStatus(status);
        if (status.running) {
          setCacheStorageMigrationModalOpen(true);
        } else if (cacheStorageMigrationWasRunningRef.current && status.status !== "idle") {
          setCacheStorageMigrationModalOpen(true);
        }
        cacheStorageMigrationWasRunningRef.current = status.running;
        if (!status.running && status.status !== "idle") {
          const settings = await api.getAppSettings();
          if (cancelled) {
            return;
          }
          state.setViewerPreviewSize(settings.viewer_preview_size);
          setCacheStorageDir(settings.cache_storage_dir ?? "");
          setSavedCacheStorageDir(settings.cache_storage_dir ?? "");
          state.setRootsInput(settings.indexed_roots.join(";"));
          const cacheStats = await api.getCacheStats();
          if (!cancelled) {
            state.setCacheStats(cacheStats);
          }
        }
      } catch (error) {
        console.error("failed to refresh cache migration status", error);
      }
    }

    void refreshMigrationStatus();
    if (!cacheStorageMigrationStatus?.running) {
      return;
    }

    const timer = window.setInterval(() => {
      void refreshMigrationStatus();
    }, 500);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [cacheStorageMigrationStatus?.running, cacheStorageMigrationStatus?.status, state, tauriRuntime]);

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

  useEffect(() => {
    if (!didInitViewerPreviewSizeEffect.current) {
      didInitViewerPreviewSizeEffect.current = true;
      return;
    }

    setThumbnailResetKey((value) => value + 1);
    setViewerPreviewReadyAssetIds([]);
  }, [state.viewerPreviewSize]);

  useEffect(() => {
    if (!tauriRuntime) {
      return;
    }
    if (!didInitRootsSettingsEffect.current) {
      didInitRootsSettingsEffect.current = true;
      return;
    }
    if (backupTransferWorking || cacheStorageMigrationStatus?.running) {
      return;
    }

    const timer = window.setTimeout(() => {
      void api
        .updateAppSettings(buildAppSettings({ indexedRoots: currentIndexedRoots() }))
        .then((settings) => {
          setSavedCacheStorageDir(settings.cache_storage_dir ?? "");
        })
        .catch((error) => {
          console.error("failed to persist indexed roots", error);
        });
    }, 400);

    return () => window.clearTimeout(timer);
  }, [
    backupTransferWorking,
    cacheStorageMigrationStatus?.running,
    cacheStorageDir,
    state.rootsInput,
    state.viewerPreviewSize,
    tauriRuntime,
  ]);

  async function startRefreshForRoots(roots: string[]) {
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
          if (
            status.status === "completed" ||
            status.status === "failed" ||
            status.status === "cancelled"
          ) {
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
      if (tauriRuntime) {
        const settings = await api.updateAppSettings(buildAppSettings({ indexedRoots: roots }));
        state.setViewerPreviewSize(settings.viewer_preview_size);
        setCacheStorageDir(settings.cache_storage_dir ?? "");
        setSavedCacheStorageDir(settings.cache_storage_dir ?? "");
        state.setRootsInput(settings.indexed_roots.join(";"));
      }
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

  async function handleRefreshIndex() {
    const roots = currentIndexedRoots();
    await startRefreshForRoots(roots);
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

  async function handleBrowseCacheStorageDir() {
    if (!tauriRuntime) {
      return;
    }

    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Choose cache storage folder",
      });

      if (!selected || Array.isArray(selected)) {
        return;
      }

      setCacheStorageDir(selected);
    } catch (error) {
      await logClient("ui.settings", `cache storage browse failed: ${String(error)}`, "error");
    }
  }

  async function handleExportBackup() {
    if (!tauriRuntime || backupTransferWorking || refreshRunning) {
      return;
    }

    try {
      const persistedSettings = await api.updateAppSettings(
        buildAppSettings({ indexedRoots: currentIndexedRoots() }),
      );
      state.setRootsInput(persistedSettings.indexed_roots.join(";"));
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Choose backup export folder",
      });
      if (!selected || Array.isArray(selected)) {
        return;
      }

      setBackupTransferMode("export");
      setBackupTransferMessage(`Exporting backup to ${selected}`);
      setBackupTransferWorking(true);
      const result = await api.exportAppBackup(selected);
      setBackupTransferMessage(
        `Export completed: ${result.cache_files} cache files, ${formatFileSize(result.cache_bytes)}`,
      );
      window.alert(
        `Backup exported to ${result.backup_dir}\n\nCache files: ${result.cache_files}\nCache size: ${formatFileSize(result.cache_bytes)}`,
      );
    } catch (error) {
      await logClient("ui.backup", `backup export failed: ${String(error)}`, "error");
      window.alert(`Backup export failed: ${String(error)}`);
    } finally {
      setBackupTransferWorking(false);
    }
  }

  async function handleOpenImportBackup() {
    if (!tauriRuntime || backupTransferWorking || refreshRunning) {
      return;
    }

    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Choose backup folder to import",
      });
      if (!selected || Array.isArray(selected)) {
        return;
      }

      const manifest = await api.inspectAppBackup(selected);
      setImportBackupDir(selected);
      setImportBackupManifest(manifest);
      setImportBackupRootsInput(
        (manifest.settings.indexed_roots.length > 0
          ? manifest.settings.indexed_roots
          : currentIndexedRoots()
        ).join(";"),
      );
      setImportBackupCacheStorageDir(manifest.settings.cache_storage_dir ?? "");
      setImportBackupShouldRefresh(true);
    } catch (error) {
      await logClient("ui.backup", `backup inspect failed: ${String(error)}`, "error");
      window.alert(`Backup import failed: ${String(error)}`);
    }
  }

  async function handleBrowseImportCacheStorageDir() {
    if (!tauriRuntime) {
      return;
    }
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Choose restored cache storage folder",
      });
      if (!selected || Array.isArray(selected)) {
        return;
      }
      setImportBackupCacheStorageDir(selected);
    } catch (error) {
      await logClient("ui.backup", `import cache browse failed: ${String(error)}`, "error");
    }
  }

  async function handleBrowseImportTakeoutRoot() {
    if (!tauriRuntime) {
      return;
    }
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Choose restored Google Photos Takeout root",
      });
      if (!selected || Array.isArray(selected)) {
        return;
      }
      setImportBackupRootsInput((current) => {
        const trimmed = current.trim();
        return trimmed ? `${trimmed};${selected}` : selected;
      });
    } catch (error) {
      await logClient("ui.backup", `import takeout browse failed: ${String(error)}`, "error");
    }
  }

  async function handleConfirmImportBackup() {
    if (!importBackupDir || !importBackupManifest) {
      return;
    }

    const roots = importBackupRootsInput
      .split(";")
      .map((item) => item.trim())
      .filter(Boolean);
    if (roots.length === 0) {
      window.alert("Provide at least one Takeout root before importing.");
      return;
    }

    try {
      setBackupTransferMode("import");
      setBackupTransferMessage(`Importing backup from ${importBackupDir}`);
      setBackupTransferWorking(true);
      const result = await api.importAppBackup(
        importBackupDir,
        roots,
        importBackupCacheStorageDir.trim() || null,
      );
      setBackupTransferMessage(
        `Import completed: ${result.cache_files} cache files restored, ${formatFileSize(result.cache_bytes)}`,
      );
      state.setViewerPreviewSize(result.settings.viewer_preview_size);
      setCacheStorageDir(result.settings.cache_storage_dir ?? "");
      setSavedCacheStorageDir(result.settings.cache_storage_dir ?? "");
      state.setRootsInput(result.settings.indexed_roots.join(";"));
      setImportBackupDir(undefined);
      setImportBackupManifest(undefined);
      setThumbnailResetKey((value) => value + 1);
      setViewerPreviewReadyAssetIds([]);
      await refreshDebugSurfaces();
      await refreshAllAssets();
      if (importBackupShouldRefresh) {
        await startRefreshForRoots(result.settings.indexed_roots);
      }
    } catch (error) {
      await logClient("ui.backup", `backup import failed: ${String(error)}`, "error");
      window.alert(`Backup import failed: ${String(error)}`);
    } finally {
      setBackupTransferWorking(false);
    }
  }

  async function handleApplyCacheStorageDir() {
    const nextValue = cacheStorageDir.trim();
    const previousValue = savedCacheStorageDir.trim();
    if (nextValue === previousValue) {
      return;
    }

    if (!tauriRuntime) {
      setSavedCacheStorageDir(nextValue);
      return;
    }

    setCacheStorageChangePending(nextValue);
  }

  async function handleConfirmCacheStorageChange(copyExisting: boolean) {
    const pendingValue = cacheStorageChangePending;
    if (pendingValue === null) {
      return;
    }

    setCacheStorageChangePending(null);

    try {
      if (copyExisting) {
        const status = await api.startCacheStorageMigration(pendingValue || null, true);
        setCacheStorageMigrationStatus(status);
        setCacheStorageMigrationModalOpen(true);
        cacheStorageMigrationWasRunningRef.current = status.running;
      } else {
        const settings = await api.updateAppSettings(
          buildAppSettings({ cacheStorageDir: pendingValue || null }),
        );
        state.setViewerPreviewSize(settings.viewer_preview_size);
        setCacheStorageDir(settings.cache_storage_dir ?? "");
        setSavedCacheStorageDir(settings.cache_storage_dir ?? "");
        state.setRootsInput(settings.indexed_roots.join(";"));
        setCacheStorageMigrationStatus({
          status: "completed",
          running: false,
          stop_requested: false,
          copy_existing: false,
          source_dir: undefined,
          destination_dir: undefined,
          total_files: 0,
          copied_files: 0,
          total_bytes: 0,
          copied_bytes: 0,
          current_path: undefined,
          message: "Switched cache storage. New assets will be rendered there.",
        });
        setCacheStorageMigrationModalOpen(true);
        cacheStorageMigrationWasRunningRef.current = false;
        state.setCacheStats(await api.getCacheStats());
      }
    } catch (error) {
      await logClient("ui.settings", `failed to change cache storage: ${String(error)}`, "error");
      window.alert(`Failed to change cache storage: ${String(error)}`);
    }
  }

  async function handleCancelCacheStorageMigration() {
    try {
      const status = await api.cancelCacheStorageMigration();
      setCacheStorageMigrationStatus(status);
    } catch (error) {
      await logClient("ui.settings", `failed to stop cache copy: ${String(error)}`, "error");
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

  async function handleViewerPreviewSizeChange(value: number) {
    if (value === state.viewerPreviewSize) {
      return;
    }

    const accepted = await confirmDestructiveAction(
      "Change viewer preview size?",
      "Changing viewer preview size will regenerate viewer previews as they are requested. When a new preview is generated, any other preview-size variant for that asset will be removed. Continue?",
      "Change Size",
    );
    if (!accepted) {
      return;
    }

    if (!tauriRuntime) {
      state.setViewerPreviewSize(value);
      return;
    }

    try {
      const settings = await api.updateAppSettings(buildAppSettings({ viewerPreviewSize: value }));
      state.setViewerPreviewSize(settings.viewer_preview_size);
      setCacheStorageDir(settings.cache_storage_dir ?? "");
      setSavedCacheStorageDir(settings.cache_storage_dir ?? "");
      state.setRootsInput(settings.indexed_roots.join(";"));
    } catch (error) {
      await logClient("ui.settings", `failed to update viewer preview size: ${String(error)}`, "error");
    }
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

  async function hydratePlaceholderPage(pageStartCursor: number) {
    const existingPage = gridPagesRef.current.find((page) => page.startCursor === pageStartCursor);
    if (!existingPage || existingPage.items !== null) {
      return;
    }
    if (hydratingPageStartsRef.current.has(pageStartCursor)) {
      return;
    }

    hydratingPageStartsRef.current.add(pageStartCursor);
    const generation = assetQueryGenerationRef.current;
    try {
      const { response, viewMode } = await fetchAssetsPage({}, pageStartCursor);
      if (generation !== assetQueryGenerationRef.current) {
        return;
      }
      setViewAssetCount(response.total_count);
      commitHydratedPage(pageStartCursor, response.items, response.next_cursor ?? null);
      await logClient(
        "ui.refresh",
        `hydrated placeholder page start=${pageStartCursor} count=${response.items.length} in ${viewMode} mode`,
      );
    } finally {
      hydratingPageStartsRef.current.delete(pageStartCursor);
    }
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
    try {
      await api.resetLocalDatabase();
      state.setAssets([]);
      state.setSelectedAsset(undefined);
      state.setAlbums([]);
      state.setDiagnostics([]);
      state.setLogs([]);
      state.setCacheStats(undefined);
      state.setImportStatus(undefined);
      state.setSelectedAlbumId(undefined);
      state.setViewMode("timeline");
      commitGridPages([]);
      setViewAssetCount(0);
      setLoadingPreviousAssets(false);
      setLoadingMoreAssets(false);
      setViewerPreviewReadyAssetIds([]);
      window.location.reload();
    } catch (error) {
      window.alert(`Clear local database failed: ${String(error)}`);
      await refreshDebugSurfaces();
      await refreshAllAssets();
    }
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
    const accepted = await confirmDestructiveAction(
      "Clear thumbnails?",
      "This will clear generated thumbnails and viewer previews from memory. Continue?",
      "Clear Thumbnails",
    );
    if (!accepted) {
      return;
    }
    await api.clearThumbnailCache();
    setThumbnailResetKey((value) => value + 1);
    setViewerPreviewReadyAssetIds([]);
    if (thumbLogOpen) {
      setThumbGenerationLogs(await api.getThumbGenerationLogs());
    }
    const cacheStats = await api.getCacheStats();
    state.setCacheStats(cacheStats);
  }

  async function handleOpenThumbLog() {
    setThumbLogOpen(true);
    const [logs, status] = await Promise.all([
      api.getThumbGenerationLogs(),
      api.getBatchThumbnailGenerationStatus(),
    ]);
    setThumbGenerationLogs(logs);
    setBatchThumbnailStatus(status);
  }

  async function handleClearThumbLog() {
    await api.clearThumbGenerationLogs();
    setThumbGenerationLogs([]);
    await refreshDebugSurfaces();
  }

  async function handleStartBatchThumbnailGeneration() {
    const status = await api.startBatchThumbnailGeneration();
    setBatchThumbnailStatus(status);
    if (thumbLogOpen) {
      setThumbGenerationLogs(await api.getThumbGenerationLogs());
    }
  }

  async function handleStopBatchThumbnailGeneration() {
    const status = await api.stopBatchThumbnailGeneration();
    setBatchThumbnailStatus(status);
  }

  async function handleCopyThumbLog() {
    const text = thumbGenerationLogs
      .map(
        (entry) => {
          const relativePath =
            entry.asset_id != null ? logAssetPaths[entry.asset_id] : undefined;
          return `${formatLogTimestamp(entry.created_at)} [${entry.level}] asset=${entry.asset_id ?? "?"} ${entry.message}${
            relativePath ? `\n${relativePath}` : ""
          }`;
        },
      )
      .join("\n");
    await navigator.clipboard.writeText(text);
  }

  async function handleClearViewerRenders() {
    const accepted = await confirmDestructiveAction(
      "Clear transcoded media?",
      "This will delete cached transcoded viewer media and completed video transcodes. Continue?",
      "Clear Transcoded Media",
    );
    if (!accepted) {
      return;
    }
    await api.clearViewerRenderCache();
    const cacheStats = await api.getCacheStats();
    state.setCacheStats(cacheStats);
  }

  async function handleStartBatchTranscode() {
    const status = await api.startBatchViewerTranscode(getViewerPlaybackSupport());
    setBatchTranscodeStatus(status);
    if (batchTranscodeOpen) {
      setBatchTranscodeLogs(await api.getBatchViewerTranscodeLogs());
    }
  }

  async function handleOpenBatchTranscode() {
    setBatchTranscodeOpen(true);
    const [status, logs] = await Promise.all([
      api.getBatchViewerTranscodeStatus(),
      api.getBatchViewerTranscodeLogs(),
    ]);
    setBatchTranscodeStatus(status);
    setBatchTranscodeLogs(logs);
  }

  async function handleStopBatchTranscode() {
    const status = await api.stopBatchViewerTranscode();
    setBatchTranscodeStatus(status);
  }

  async function handleClearBatchTranscodeLog() {
    await api.clearBatchViewerTranscodeLogs();
    setBatchTranscodeLogs([]);
  }

  async function handleCopyBatchTranscodeLog() {
    const text = batchTranscodeLogs
      .map(
        (entry) => {
          const relativePath =
            entry.asset_id != null ? logAssetPaths[entry.asset_id] : undefined;
          return `${formatLogTimestamp(entry.created_at)} ${entry.message}${
            relativePath ? `\n${relativePath}` : ""
          }`;
        },
      )
      .join("\n");
    await navigator.clipboard.writeText(text);
  }

  async function handleCopyAppLogs() {
    const text = state.logs
      .map(
        (entry) =>
          `${formatLogTimestamp(entry.created_at)} [${entry.level}] ${entry.scope}${
            entry.asset_id != null ? ` asset=${entry.asset_id}` : ""
          } ${entry.message}`,
      )
      .join("\n");
    await navigator.clipboard.writeText(text);
  }

  useEffect(() => {
    const targetLogs = [
      ...(thumbLogOpen ? thumbGenerationLogs : []),
      ...(batchTranscodeOpen ? batchTranscodeLogs : []),
    ];
    const assetIds = [...new Set(targetLogs.map((entry) => entry.asset_id).filter((value): value is number => value != null))];
    const missingIds = assetIds.filter((assetId) => !logAssetPaths[assetId]);
    if (missingIds.length === 0) {
      return;
    }

    let cancelled = false;
    void Promise.all(
      missingIds.map(async (assetId) => {
        try {
          const detail = await api.getAssetDetail(assetId);
          const relativePath = detail.primary_path
            ? formatPathRelativeToRoots(detail.primary_path, currentIndexedRoots())
            : undefined;
          return relativePath ? [assetId, relativePath] as const : undefined;
        } catch {
          return undefined;
        }
      }),
    ).then((entries) => {
      if (cancelled) {
        return;
      }
      const resolved = Object.fromEntries(entries.filter((entry): entry is readonly [number, string] => Boolean(entry)));
      if (Object.keys(resolved).length === 0) {
        return;
      }
      setLogAssetPaths((current) => ({ ...current, ...resolved }));
    });

    return () => {
      cancelled = true;
    };
  }, [batchTranscodeLogs, batchTranscodeOpen, logAssetPaths, state.rootsInput, thumbGenerationLogs, thumbLogOpen]);

  function handleViewerPreviewReady(assetId: number) {
    setViewerPreviewReadyAssetIds((current) =>
      current.includes(assetId) ? current : [...current, assetId],
    );
  }

  const selectedAssetIndex = state.selectedAsset
    ? state.assets.findIndex((asset) => asset.id === state.selectedAsset?.id)
    : -1;

  return (
    <div className={`app-shell${debugPanelCollapsed ? " debug-collapsed" : ""}`}>
      <Sidebar
        rootsInput={state.rootsInput}
        indexedRoots={currentIndexedRoots()}
        viewerPreviewSize={state.viewerPreviewSize}
        cacheStorageDir={cacheStorageDir}
        settingsCollapsed={state.settingsCollapsed}
        cacheStorageBusy={cacheStorageMigrationStatus?.running}
        importStatus={state.importStatus}
        refreshRunning={refreshRunning}
        browseEnabled={tauriRuntime}
        albums={state.albums}
        selectedAlbumId={state.selectedAlbumId}
        onRootsInputChange={state.setRootsInput}
        onViewerPreviewSizeChange={(value) => void handleViewerPreviewSizeChange(value)}
        onCacheStorageDirChange={setCacheStorageDir}
        onApplyCacheStorageDir={() => void handleApplyCacheStorageDir()}
        onBrowseCacheStorageDir={() => void handleBrowseCacheStorageDir()}
        onResetCacheStorageDir={() => setCacheStorageDir("")}
        onToggleSettingsCollapsed={() => state.setSettingsCollapsed(!state.settingsCollapsed)}
        onBrowseRoot={handleBrowseRoot}
        onRefresh={handleRefreshIndex}
        onExportBackup={() => void handleExportBackup()}
        onImportBackup={() => void handleOpenImportBackup()}
        onResetDatabase={handleResetDatabase}
        onShowTimeline={handleShowTimeline}
        onSelectAlbum={handleSelectAlbum}
      />

      <main className="panel content-panel">
        <Toolbar
          query={state.query}
          mediaKind={state.mediaKind}
          timelineLabel={timelineLabel}
          assetCount={viewAssetCount}
          onQueryChange={state.setQuery}
          onMediaKindChange={state.setMediaKind}
        />
        <div className="grid-frame">
          <MediaGrid
            assets={state.assets}
            viewMode={state.viewMode}
            entries={gridEntries}
            viewerPreviewSize={state.viewerPreviewSize}
            onSelect={handleSelectAsset}
            onHydratePlaceholderPage={hydratePlaceholderPage}
            viewerPlaybackSupport={viewerPlaybackSupport}
            viewerPreviewReadyAssetIds={viewerPreviewReadyAssetIds}
            thumbnailResetKey={thumbnailResetKey}
            hasMoreBefore={hasPreviousAssetPage}
            hasMoreAfter={hasNextAssetPage}
            isLoadingMoreBefore={loadingPreviousAssets}
            isLoadingMore={loadingMoreAssets}
            onLoadMoreBefore={loadPreviousAssets}
            onLoadMore={loadMoreAssets}
            onLeadingDateChange={(value) => {
              setTimelineLabel(formatTimelineLabel(value));
            }}
          />
        </div>
      </main>

      <DebugPanel
        diagnostics={state.diagnostics}
        logs={state.logs}
        cacheStats={state.cacheStats}
        collapsed={debugPanelCollapsed}
        thumbBatchRunning={batchThumbnailStatus?.status === "running"}
        thumbBatchStopping={batchThumbnailStatus?.status === "running" && batchThumbnailStatus?.stop_requested}
        videoBatchRunning={batchTranscodeStatus?.status === "running"}
        videoBatchStopping={batchTranscodeStatus?.status === "running" && batchTranscodeStatus?.stop_requested}
        onToggleCollapsed={() => setDebugPanelCollapsed((current) => !current)}
        onStartThumbBatch={() => void handleStartBatchThumbnailGeneration()}
        onStopThumbBatch={() => void handleStopBatchThumbnailGeneration()}
        onOpenThumbLog={() => void handleOpenThumbLog()}
        onStartBatchTranscode={() => void handleStartBatchTranscode()}
        onStopBatchTranscode={() => void handleStopBatchTranscode()}
        onOpenBatchTranscode={() => void handleOpenBatchTranscode()}
        onOpenDiagnostics={() => setDiagnosticsOpen(true)}
        onOpenAppLogs={() => setAppLogsOpen(true)}
        onClearThumbnails={handleClearThumbnails}
        onClearViewerRenders={handleClearViewerRenders}
      />

      {appLogsOpen ? (
        <div className="viewer-backdrop" onClick={() => setAppLogsOpen(false)}>
          <div className="thumb-log-card" onClick={(event) => event.stopPropagation()}>
            <div className="viewer-toolbar">
              <div>
                <div className="title">App Logs</div>
                <div className="muted">
                  General app activity, refresh summaries, cache actions, and warnings.
                </div>
              </div>
              <div className="button-row">
                <button className="button-secondary" onClick={() => void handleCopyAppLogs()}>
                  Copy
                </button>
                <button className="button-danger" onClick={handleClearLogs}>
                  Clear
                </button>
                <button className="button-danger" onClick={() => setAppLogsOpen(false)}>
                  Close
                </button>
              </div>
            </div>
            <div className="viewer-meta">
              <div className="status-banner">{state.logs.length} app log entries</div>
            </div>
            <div className="thumb-log-list">
              {state.logs.length > 0 ? (
                state.logs.map((entry) => (
                  <div key={entry.id} className="thumb-log-line thumb-log-line-detailed">
                    <span className="thumb-log-timestamp">{formatLogTimestamp(entry.created_at)}</span>
                    <span className="thumb-log-message">
                      <span className="thumb-log-main-message">
                        [{entry.level}] {entry.scope}
                        {entry.asset_id != null ? ` • asset ${entry.asset_id}` : ""} • {entry.message}
                      </span>
                    </span>
                  </div>
                ))
              ) : (
                <div className="empty-state">No app log entries recorded.</div>
              )}
            </div>
          </div>
        </div>
      ) : null}

      {diagnosticsOpen ? (
        <div className="viewer-backdrop" onClick={() => setDiagnosticsOpen(false)}>
          <div className="thumb-log-card" onClick={(event) => event.stopPropagation()}>
            <div className="viewer-toolbar">
              <div>
                <div className="title">Ingress Diagnostics</div>
                <div className="muted">
                  Import warnings and unresolved sidecar or matching issues.
                </div>
              </div>
              <div className="button-row">
                <button className="button-danger" onClick={handleClearDiagnostics}>
                  Clear
                </button>
                <button className="button-danger" onClick={() => setDiagnosticsOpen(false)}>
                  Close
                </button>
              </div>
            </div>
            <div className="viewer-meta">
              <div className="status-banner">
                {state.diagnostics.length} warning{state.diagnostics.length === 1 ? "" : "s"}
              </div>
            </div>
            <div className="thumb-log-list">
              {state.diagnostics.length > 0 ? (
                state.diagnostics.map((diagnostic) => (
                  <div key={diagnostic.id} className="thumb-log-line thumb-log-line-detailed">
                    <span className="thumb-log-timestamp">{formatLogTimestamp(diagnostic.created_at)}</span>
                    <span className="thumb-log-message">
                      <span className="thumb-log-main-message">
                        [{diagnostic.severity}] {diagnostic.diagnostic_type} • import {diagnostic.import_id} • {diagnostic.message}
                      </span>
                      {diagnostic.related_path ? (
                        <span className="thumb-log-main-message">{diagnostic.related_path}</span>
                      ) : null}
                    </span>
                  </div>
                ))
              ) : (
                <div className="empty-state">No ingress diagnostics recorded.</div>
              )}
            </div>
          </div>
        </div>
      ) : null}

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
                <button
                  className={`button-primary${batchThumbnailStatus?.status === "running" ? " button-working" : ""}`}
                  onClick={() => void handleStartBatchThumbnailGeneration()}
                  disabled={batchThumbnailStatus?.status === "running"}
                >
                  {batchThumbnailStatus?.status === "running" ? "Working" : "Start"}
                </button>
                <button
                  className="button-danger"
                  onClick={() => void handleStopBatchThumbnailGeneration()}
                  disabled={!batchThumbnailStatus?.status || batchThumbnailStatus.status !== "running"}
                >
                  Stop After Current
                </button>
                <button className="button-secondary" onClick={() => void handleCopyThumbLog()}>
                  Copy
                </button>
                <button className="button-danger" onClick={() => void handleClearThumbLog()}>
                  Clear
                </button>
                <button className="button-danger" onClick={() => setThumbLogOpen(false)}>
                  Close
                </button>
              </div>
            </div>
            <div className="viewer-meta">
              <div className="status-banner">{formatThumbnailBatchStatusLine(batchThumbnailStatus)}</div>
            </div>
            <div className="thumb-log-list">
              {thumbGenerationLogs.length > 0 ? (
                thumbGenerationLogs.map((entry) => {
                  const parsed = parseThumbLogMessage(entry.message);
                  return (
                    <div key={entry.id} className="thumb-log-line thumb-log-line-detailed">
                      <span className="thumb-log-timestamp">{formatLogTimestamp(entry.created_at)}</span>
                      <span className="thumb-log-message">
                        <span className="thumb-log-main-message">
                          [{entry.level}] asset={entry.asset_id ?? "?"} {parsed.baseMessage}
                        </span>
                        {entry.asset_id != null && logAssetPaths[entry.asset_id] ? (
                          <span className="thumb-log-main-message">{logAssetPaths[entry.asset_id]}</span>
                        ) : null}
                        {parsed.metrics.length > 0 ? (
                          <span className="thumb-log-metrics">
                            {parsed.metrics.map((metric) => (
                              <span key={`${entry.id}-${metric.label}`} className="thumb-log-metric-chip">
                                <strong>{metric.label}</strong> {metric.value}
                              </span>
                            ))}
                          </span>
                        ) : null}
                      </span>
                    </div>
                  );
                })
              ) : (
                <div className="empty-state">No thumbnail generation events recorded yet.</div>
              )}
            </div>
          </div>
        </div>
      ) : null}

      {batchTranscodeOpen ? (
        <div className="viewer-backdrop" onClick={() => setBatchTranscodeOpen(false)}>
          <div className="thumb-log-card" onClick={(event) => event.stopPropagation()}>
            <div className="viewer-toolbar">
              <div>
                <div className="title">Batch Video Transcode</div>
                <div className="muted">
                  Pre-renders viewer-safe HEVC files for the indexed videos.
                </div>
              </div>
              <div className="button-row">
                <button
                  className={`button-primary${batchTranscodeStatus?.status === "running" ? " button-working" : ""}`}
                  onClick={() => void handleStartBatchTranscode()}
                  disabled={batchTranscodeStatus?.status === "running"}
                >
                  {batchTranscodeStatus?.status === "running" ? "Working" : "Start"}
                </button>
                <button
                  className="button-danger"
                  onClick={() => void handleStopBatchTranscode()}
                  disabled={!batchTranscodeStatus?.status || batchTranscodeStatus.status !== "running"}
                >
                  Stop After Current
                </button>
                <button className="button-secondary" onClick={() => void handleCopyBatchTranscodeLog()}>
                  Copy Log
                </button>
                <button className="button-danger" onClick={() => void handleClearBatchTranscodeLog()}>
                  Clear Log
                </button>
                <button className="button-danger" onClick={() => setBatchTranscodeOpen(false)}>
                  Close
                </button>
              </div>
            </div>
            <div className="viewer-meta">
              <div className="status-banner">{formatBatchStatusLine(batchTranscodeStatus)}</div>
              {batchTranscodeStatus?.current_filename ? (
                <div style={{ marginTop: 16 }}>
                  <strong>Current file</strong>
                  <div className="muted">{batchTranscodeStatus.current_filename}</div>
                </div>
              ) : null}
              {batchTranscodeStatus?.current_codec ? (
                <div className="muted" style={{ marginTop: 8 }}>
                  Source codec {batchTranscodeStatus.current_codec}
                </div>
              ) : null}
              {typeof batchTranscodeStatus?.current_width === "number" &&
              typeof batchTranscodeStatus?.current_height === "number" ? (
                <div className="muted" style={{ marginTop: 8 }}>
                  Resolution {batchTranscodeStatus.current_width}x{batchTranscodeStatus.current_height}
                </div>
              ) : null}
              {typeof batchTranscodeStatus?.current_duration_ms === "number" &&
              batchTranscodeStatus.current_duration_ms > 0 ? (
                <div className="muted" style={{ marginTop: 8 }}>
                  Duration {formatMediaDurationMs(batchTranscodeStatus.current_duration_ms)}
                </div>
              ) : null}
              {typeof batchTranscodeStatus?.current_elapsed_ms === "number" ? (
                <div className="muted" style={{ marginTop: 8 }}>
                  Current elapsed {(batchTranscodeStatus.current_elapsed_ms / 1000).toFixed(1)}s
                </div>
              ) : null}
              {typeof batchTranscodeStatus?.current_source_bytes === "number" ? (
                <div className="muted" style={{ marginTop: 8 }}>
                  Source {formatFileSize(batchTranscodeStatus.current_source_bytes)}
                  {typeof batchTranscodeStatus.current_output_bytes === "number"
                    ? ` • Written ${formatFileSize(batchTranscodeStatus.current_output_bytes)}`
                    : ""}
                </div>
              ) : null}
            </div>
            <div className="thumb-log-list">
              {batchTranscodeLogs.length > 0 ? (
                batchTranscodeLogs.map((entry) => (
                  <div key={entry.id} className="thumb-log-line">
                    <span className="thumb-log-timestamp">{formatLogTimestamp(entry.created_at)}</span>
                    <span className="thumb-log-message">
                      <span className="thumb-log-main-message">{entry.message}</span>
                      {entry.asset_id != null && logAssetPaths[entry.asset_id] ? (
                        <span className="thumb-log-main-message">{logAssetPaths[entry.asset_id]}</span>
                      ) : null}
                    </span>
                  </div>
                ))
              ) : (
                <div className="empty-state">No batch transcode events recorded yet.</div>
              )}
            </div>
          </div>
        </div>
      ) : null}

      {cacheStorageChangePending !== null ? (
        <div className="viewer-backdrop" onClick={() => setCacheStorageChangePending(null)}>
          <div className="thumb-log-card" onClick={(event) => event.stopPropagation()}>
            <div className="viewer-toolbar">
              <div>
                <div className="title">Move Cache Storage?</div>
                <div className="muted">
                  Existing thumbnails, previews, and rendered viewer media can be copied to the new
                  location instead of being regenerated later.
                </div>
              </div>
            </div>
            <div className="viewer-meta">
              <div className="status-banner">
                New location: {cacheStorageChangePending || "default app support folder"}
              </div>
            </div>
            <div className="button-row" style={{ marginTop: 16 }}>
              <button className="button-primary" onClick={() => void handleConfirmCacheStorageChange(true)}>
                Copy Existing Cache
              </button>
              <button className="button-secondary" onClick={() => void handleConfirmCacheStorageChange(false)}>
                Use Empty Location
              </button>
              <button className="button-danger" onClick={() => setCacheStorageChangePending(null)}>
                Cancel
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {cacheStorageMigrationModalOpen &&
      cacheStorageMigrationStatus &&
      cacheStorageMigrationStatus.status !== "idle" ? (
        <div
          className="viewer-backdrop"
          onClick={() => {
            if (!cacheStorageMigrationStatus.running) {
              setCacheStorageMigrationModalOpen(false);
            }
          }}
        >
          <div className="thumb-log-card" onClick={(event) => event.stopPropagation()}>
            <div className="viewer-toolbar">
              <div>
                <div className="title">Cache Storage Copy</div>
                <div className="muted">
                  {cacheStorageMigrationStatus.copy_existing
                    ? "Moving generated cache assets to the new storage location."
                    : "Updating cache storage location."}
                </div>
              </div>
              <div className="button-row">
                {cacheStorageMigrationStatus.running ? (
                  <button className="button-secondary" onClick={() => void handleCancelCacheStorageMigration()}>
                    Interrupt
                  </button>
                ) : null}
                <button
                  className="button-danger"
                  onClick={() => setCacheStorageMigrationModalOpen(false)}
                  disabled={cacheStorageMigrationStatus.running}
                >
                  Close
                </button>
              </div>
            </div>
            <div className="viewer-meta">
              <div className="status-banner">
                {formatCacheStorageMigrationLine(cacheStorageMigrationStatus)}
              </div>
              {cacheStorageMigrationStatus.destination_dir ? (
                <div style={{ marginTop: 16 }}>
                  <strong>Destination</strong>
                  <div className="muted">{cacheStorageMigrationStatus.destination_dir}</div>
                </div>
              ) : null}
              {cacheStorageMigrationStatus.current_path ? (
                <div className="muted" style={{ marginTop: 8 }}>
                  Current file {cacheStorageMigrationStatus.current_path}
                </div>
              ) : null}
              {cacheStorageMigrationStatus.total_bytes > 0 ? (
                <div className="muted" style={{ marginTop: 8 }}>
                  {formatFileSize(cacheStorageMigrationStatus.copied_bytes)} /{" "}
                  {formatFileSize(cacheStorageMigrationStatus.total_bytes)}
                </div>
              ) : null}
            </div>
          </div>
        </div>
      ) : null}

      {importBackupManifest && importBackupDir ? (
        <div
          className="viewer-backdrop"
          onClick={() => {
            if (!backupTransferWorking) {
              setImportBackupDir(undefined);
              setImportBackupManifest(undefined);
            }
          }}
        >
          <div className="thumb-log-card" onClick={(event) => event.stopPropagation()}>
            <div className="viewer-toolbar">
              <div>
                <div className="title">Import Backup</div>
                <div className="muted">
                  Restore the exported database, settings, and caches from {importBackupDir}.
                </div>
              </div>
            </div>
            <div className="viewer-meta">
              <div className="status-banner">
                Backup from {formatLogTimestamp(importBackupManifest.exported_at)} •{" "}
                {importBackupManifest.settings.indexed_roots.length} Takeout root
                {importBackupManifest.settings.indexed_roots.length === 1 ? "" : "s"}
              </div>
              <div className="setting-row" style={{ marginTop: 16 }}>
                <label className="setting-label" htmlFor="import-backup-roots">
                  Takeout roots
                </label>
                <input
                  id="import-backup-roots"
                  value={importBackupRootsInput}
                  onChange={(event) => setImportBackupRootsInput(event.target.value)}
                  placeholder="/path/to/Takeout/Google Photos;/another/root"
                  disabled={backupTransferWorking}
                />
              </div>
              <div className="button-row" style={{ marginTop: 8 }}>
                <button
                  className="button-secondary"
                  onClick={() => void handleBrowseImportTakeoutRoot()}
                  disabled={backupTransferWorking}
                >
                  Browse Takeout Root
                </button>
              </div>
              <div className="muted">
                Point this backup at the current location of the original Takeout files.
              </div>
              <div className="setting-row" style={{ marginTop: 16 }}>
                <label className="setting-label" htmlFor="import-backup-cache-dir">
                  Cache storage location
                </label>
                <input
                  id="import-backup-cache-dir"
                  value={importBackupCacheStorageDir}
                  onChange={(event) => setImportBackupCacheStorageDir(event.target.value)}
                  placeholder="Leave blank to restore into the default app support folder"
                  disabled={backupTransferWorking}
                />
              </div>
              <div className="button-row" style={{ marginTop: 8 }}>
                <button
                  className="button-secondary"
                  onClick={() => void handleBrowseImportCacheStorageDir()}
                  disabled={backupTransferWorking}
                >
                  Browse Cache Folder
                </button>
                <button
                  className="button-secondary"
                  onClick={() => setImportBackupCacheStorageDir("")}
                  disabled={backupTransferWorking}
                >
                  Use Default
                </button>
              </div>
              <label
                style={{
                  display: "flex",
                  gap: 8,
                  alignItems: "center",
                  marginTop: 16,
                  cursor: backupTransferWorking ? "default" : "pointer",
                }}
              >
                <input
                  type="checkbox"
                  checked={importBackupShouldRefresh}
                  onChange={(event) => setImportBackupShouldRefresh(event.target.checked)}
                  disabled={backupTransferWorking}
                />
                <span>
                  Run refresh after import
                  <span className="muted"> (recommended)</span>
                </span>
              </label>
            </div>
            <div className="button-row" style={{ marginTop: 16 }}>
              <button
                className="button-primary"
                onClick={() => void handleConfirmImportBackup()}
                disabled={backupTransferWorking}
              >
                {backupTransferWorking ? "Working" : "Import Backup"}
              </button>
              <button
                className="button-danger"
                onClick={() => {
                  if (!backupTransferWorking) {
                    setImportBackupDir(undefined);
                    setImportBackupManifest(undefined);
                  }
                }}
                disabled={backupTransferWorking}
              >
                Cancel
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {backupTransferWorking ? (
        <div className="viewer-backdrop">
          <div className="thumb-log-card" onClick={(event) => event.stopPropagation()}>
            <div className="viewer-toolbar">
              <div>
                <div className="title">
                  {backupTransferMode === "export" ? "Exporting Backup" : "Importing Backup"}
                </div>
                <div className="muted">
                  {backupTransferMode === "export"
                    ? "Packaging the database, settings, and caches."
                    : "Restoring the database, settings, and caches."}
                </div>
              </div>
            </div>
            <div className="viewer-meta">
              <div className="status-banner">
                {backupTransferMessage || "Working..."}
              </div>
            </div>
          </div>
        </div>
      ) : null}

      <ViewerModal
        asset={state.selectedAsset}
        viewerPreviewSize={state.viewerPreviewSize}
        hasPrevious={selectedAssetIndex > 0}
        hasNext={selectedAssetIndex >= 0 && selectedAssetIndex < state.assets.length - 1}
        onPrevious={() => void handleStepAsset(-1)}
        onNext={() => void handleStepAsset(1)}
        onClose={() => state.setSelectedAsset(undefined)}
        onViewerPreviewReady={handleViewerPreviewReady}
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

function formatFileSize(bytes: number) {
  if (bytes >= 1024 * 1024 * 1024) {
    return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
  }
  if (bytes >= 1024 * 1024) {
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
  if (bytes >= 1024) {
    return `${(bytes / 1024).toFixed(1)} kB`;
  }
  return `${bytes} B`;
}

function formatBatchStatusLine(status?: BatchViewerTranscodeStatus) {
  if (!status) {
    return "No batch transcode job has started yet.";
  }
  const processed = status.completed + status.failed;
  const parts = [
    status.status,
    `${processed}/${status.total} processed`,
    `${status.succeeded} succeeded`,
  ];
  if (status.skipped > 0) {
    parts.push(`${status.skipped} skipped`);
  }
  if (status.failed > 0) {
    parts.push(`${status.failed} failed`);
  }
  if (status.stop_requested) {
    parts.push("stop requested");
  }
  if (typeof status.elapsed_ms === "number") {
    parts.push(`${(status.elapsed_ms / 1000).toFixed(1)}s elapsed`);
  }
  if (status.message) {
    parts.push(status.message);
  }
  return parts.join(" • ");
}

function formatThumbnailBatchStatusLine(status?: BatchThumbnailGenerationStatus) {
  if (!status) {
    return "No thumbnail generation job has started yet.";
  }
  const processed = status.completed + status.failed;
  const parts = [status.status, `${processed}/${status.total} processed`];
  if (status.skipped > 0) {
    parts.push(`${status.skipped} skipped`);
  }
  if (status.failed > 0) {
    parts.push(`${status.failed} failed`);
  }
  if (status.stop_requested) {
    parts.push("stop requested");
  }
  if (typeof status.elapsed_ms === "number") {
    parts.push(`${(status.elapsed_ms / 1000).toFixed(1)}s elapsed`);
  }
  if (status.message) {
    parts.push(status.message);
  }
  return parts.join(" • ");
}

function formatCacheStorageMigrationLine(status?: CacheStorageMigrationStatus) {
  if (!status) {
    return "No cache storage copy in progress.";
  }

  const parts: string[] = [status.status];
  if (status.total_files > 0) {
    parts.push(`${status.copied_files}/${status.total_files} files`);
  }
  if (status.stop_requested) {
    parts.push("stop requested");
  }
  if (status.message) {
    parts.push(status.message);
  }
  return parts.join(" • ");
}

function parseThumbLogMessage(message: string) {
  const metricsMatch = message.match(/\smetrics="([^"]*)"/);
  const baseMessage = message.replace(/\smetrics="[^"]*"/, "");
  const metrics = new Map<string, string>();

  for (const key of ["queue_elapsed", "elapsed", "total_elapsed"]) {
    const match = baseMessage.match(new RegExp(`${key}=([^ ]+)`));
    if (match) {
      metrics.set(formatMetricLabel(key), match[1]);
    }
  }

  if (metricsMatch) {
    for (const token of metricsMatch[1].split(/\s+/)) {
      const separatorIndex = token.indexOf("=");
      if (separatorIndex <= 0) {
        continue;
      }
      const label = token.slice(0, separatorIndex);
      const value = token.slice(separatorIndex + 1);
      metrics.set(formatMetricLabel(label), value);
    }
  }

  return {
    baseMessage,
    metrics: Array.from(metrics.entries()).map(([label, value]) => ({ label, value })),
  };
}

function formatMetricLabel(label: string) {
  return label.replace(/_/g, " ");
}

function formatMediaDurationMs(durationMs: number) {
  const totalSeconds = Math.max(0, Math.round(durationMs / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

function formatPathRelativeToRoots(sourcePath: string, indexedRoots: string[]) {
  const normalizedRoots = indexedRoots
    .map((root) => root.trim())
    .filter(Boolean)
    .sort((left, right) => right.length - left.length);

  for (const root of normalizedRoots) {
    if (sourcePath === root) {
      return ".";
    }
    if (sourcePath.startsWith(`${root}/`)) {
      return sourcePath.slice(root.length + 1);
    }
  }

  return sourcePath;
}

function getViewerPlaybackSupport(): ViewerPlaybackSupport {
  if (typeof document === "undefined") {
    return {
      mp4_h264: false,
      mp4_hevc: false,
      mov_h264: false,
      mov_hevc: false,
      webm: false,
    };
  }
  const probe = document.createElement("video");
  const playable = (value: string) => probe.canPlayType(value) !== "";
  return {
    mp4_h264: playable('video/mp4; codecs="avc1.42E01E, mp4a.40.2"'),
    mp4_hevc: playable('video/mp4; codecs="hvc1.1.6.L93.B0, mp4a.40.2"'),
    mov_h264: playable('video/quicktime; codecs="avc1.42E01E, mp4a.40.2"'),
    mov_hevc: playable('video/quicktime; codecs="hvc1.1.6.L93.B0, mp4a.40.2"'),
    webm: playable("video/webm"),
  };
}
