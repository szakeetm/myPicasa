import { create } from "zustand";

import type {
  AlbumSummary,
  AssetDetail,
  AssetListItem,
  CacheStats,
  DiagnosticEntry,
  ImportProgress,
  LogEntry,
} from "../lib/types";

type ViewMode = "timeline" | "album";

type AppState = {
  rootsInput: string;
  query: string;
  mediaKind: string;
  dateFrom: string;
  dateTo: string;
  viewMode: ViewMode;
  selectedAlbumId?: number;
  albums: AlbumSummary[];
  assets: AssetListItem[];
  selectedAsset?: AssetDetail;
  diagnostics: DiagnosticEntry[];
  logs: LogEntry[];
  cacheStats?: CacheStats;
  importStatus?: ImportProgress | null;
  setRootsInput: (value: string) => void;
  setQuery: (value: string) => void;
  setMediaKind: (value: string) => void;
  setDateFrom: (value: string) => void;
  setDateTo: (value: string) => void;
  setViewMode: (value: ViewMode) => void;
  setSelectedAlbumId: (value?: number) => void;
  setAlbums: (value: AlbumSummary[]) => void;
  setAssets: (value: AssetListItem[]) => void;
  setSelectedAsset: (value?: AssetDetail) => void;
  setDiagnostics: (value: DiagnosticEntry[]) => void;
  setLogs: (value: LogEntry[]) => void;
  setCacheStats: (value?: CacheStats) => void;
  setImportStatus: (value?: ImportProgress | null) => void;
};

export const useAppState = create<AppState>((set) => ({
  rootsInput: "",
  query: "",
  mediaKind: "",
  dateFrom: "",
  dateTo: "",
  viewMode: "timeline",
  albums: [],
  assets: [],
  diagnostics: [],
  logs: [],
  setRootsInput: (rootsInput) => set({ rootsInput }),
  setQuery: (query) => set({ query }),
  setMediaKind: (mediaKind) => set({ mediaKind }),
  setDateFrom: (dateFrom) => set({ dateFrom }),
  setDateTo: (dateTo) => set({ dateTo }),
  setViewMode: (viewMode) => set({ viewMode }),
  setSelectedAlbumId: (selectedAlbumId) => set({ selectedAlbumId }),
  setAlbums: (albums) => set({ albums }),
  setAssets: (assets) => set({ assets }),
  setSelectedAsset: (selectedAsset) => set({ selectedAsset }),
  setDiagnostics: (diagnostics) => set({ diagnostics }),
  setLogs: (logs) => set({ logs }),
  setCacheStats: (cacheStats) => set({ cacheStats }),
  setImportStatus: (importStatus) => set({ importStatus }),
}));
