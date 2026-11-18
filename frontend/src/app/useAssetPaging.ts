import { useCallback, useMemo, useRef, useState } from "react";

import { logClient } from "../lib/logger";
import { api } from "../lib/tauri";
import type { AssetListRequest } from "../lib/types";
import { useAppState } from "../state/appState";
import { commitPage, type GridPage } from "./gridPaging";

const PAGE_SIZE = 200;

type AssetQuery = {
  viewMode: "timeline" | "album";
  selectedAlbumId?: number;
  query: string;
  mediaKind: string;
};

function fetchPage(query: AssetQuery, cursor?: number) {
  const request: AssetListRequest = {
    cursor,
    limit: PAGE_SIZE,
    query: query.query || undefined,
    media_kind: query.mediaKind || undefined,
  };

  if (query.viewMode === "album" && query.selectedAlbumId) {
    return api.listAssetsByAlbum(query.selectedAlbumId, request);
  }
  return query.query ? api.searchAssets(request) : api.listAssetsByDate(request);
}

function makePage(startCursor: number, response: Awaited<ReturnType<typeof fetchPage>>): GridPage {
  return {
    startCursor,
    count: response.items.length,
    nextCursor: response.next_cursor ?? null,
    items: response.items,
  };
}

export function useAssetPaging() {
  const [pages, setPages] = useState<GridPage[]>([]);
  const [totalCount, setTotalCount] = useState(0);
  const [loadingMore, setLoadingMore] = useState(false);
  const [viewVersion, setViewVersion] = useState(0);
  const active = useRef<{
    query: AssetQuery;
    inFlight: Set<number>;
    ready: boolean;
    viewerPage?: GridPage;
  }>(null);
  const assets = useMemo(() => pages.flatMap((page) => page.items ?? []), [pages]);
  const lastPage = pages.at(-1);

  const clear = useCallback(() => {
    active.current = null;
    setPages([]);
    setTotalCount(0);
    setViewVersion((version) => version + 1);
    setLoadingMore(false);
  }, []);

  const refresh = useCallback(async (options?: Partial<AssetQuery>) => {
    const { viewMode, selectedAlbumId, query, mediaKind } = useAppState.getState();
    const view = { query: { viewMode, selectedAlbumId, query, mediaKind, ...options }, inFlight: new Set<number>(), ready: false };
    const previous = active.current;
    active.current = view;
    setLoadingMore(false);
    const response = await fetchPage(view.query).catch((error) => {
      if (active.current === view) active.current = previous?.ready ? previous : null;
      throw error;
    });
    if (active.current !== view) return;
    view.ready = true;
    setPages([makePage(0, response)]);
    setViewVersion((version) => version + 1);
    setTotalCount(response.total_count);
    return response;
  }, []);

  const requestPage = useCallback(async (cursor: number, append: boolean) => {
    const view = active.current;
    if (!view?.ready || view.inFlight.has(cursor)) return;
    view.inFlight.add(cursor);
    if (append) setLoadingMore(true);
    try {
      const response = await fetchPage(view.query, cursor);
      if (active.current !== view) return;
      setPages((current) => commitPage(current, makePage(cursor, response)));
      setTotalCount(response.total_count);
    } catch (error) {
      void logClient("ui.refresh", `failed to load page starting at ${cursor}: ${String(error)}`, "error");
    } finally {
      view.inFlight.delete(cursor);
      if (append && active.current === view) setLoadingMore(false);
    }
  }, []);

  const loadMore = useCallback(() => {
    if (lastPage?.nextCursor != null) return requestPage(lastPage.nextCursor, true);
  }, [lastPage?.nextCursor, requestPage]);

  const hydrate = useCallback((cursor: number) => {
    if (pages.some((page) => page.startCursor === cursor && page.items === null)) {
      return requestPage(cursor, false);
    }
  }, [pages, requestPage]);

  const readAsset = useCallback(async (index: number) => {
    const view = active.current;
    if (!view?.ready) return;
    const cursor = Math.floor(index / PAGE_SIZE) * PAGE_SIZE;
    const page = pages.find((page) => page.startCursor === cursor && page.items) ??
      (view.viewerPage?.startCursor === cursor ? view.viewerPage : undefined);
    if (page) return page.items?.[index - cursor];

    const response = await fetchPage(view.query, cursor);
    if (active.current !== view) return;
    view.viewerPage = makePage(cursor, response);
    return response.items[index - cursor];
  }, [pages]);

  return { pages, assets, totalCount, viewVersion, readAsset, hasMore: lastPage?.nextCursor != null, loadingMore, refresh, loadMore, hydrate, clear };
}
