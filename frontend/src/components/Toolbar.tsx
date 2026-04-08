type ToolbarProps = {
  query: string;
  mediaKind: string;
  timelineLabel?: string;
  thumbnailPreloadActive: boolean;
  thumbnailPreloadProgress?: {
    thumbsCompleted: number;
    thumbsTotal: number;
    previewsCompleted: number;
    previewsTotal: number;
  };
  onQueryChange: (value: string) => void;
  onMediaKindChange: (value: string) => void;
  onToggleThumbnailPreload: () => void;
};

export function Toolbar({
  query,
  mediaKind,
  timelineLabel,
  thumbnailPreloadActive,
  thumbnailPreloadProgress,
  onQueryChange,
  onMediaKindChange,
  onToggleThumbnailPreload,
}: ToolbarProps) {
  return (
    <div className="toolbar">
      <input
        value={query}
        onChange={(event) => onQueryChange(event.target.value)}
        placeholder="Search filename, album, camera, date"
      />
      <select value={mediaKind} onChange={(event) => onMediaKindChange(event.target.value)}>
        <option value="">All media</option>
        <option value="photo">Photos</option>
        <option value="video">Videos</option>
      </select>
      <div className="timeline-marker" aria-live="polite">
        {timelineLabel || "Timeline"}
      </div>
      <button className="button-secondary" onClick={onToggleThumbnailPreload}>
        {thumbnailPreloadActive ? "Interrupt Fill" : "Generate Thumbs + Previews"}
      </button>
      <div className="timeline-marker" aria-live="polite">
        {thumbnailPreloadProgress
          ? `${thumbnailPreloadProgress.thumbsCompleted}/${thumbnailPreloadProgress.thumbsTotal} thumbs • ${thumbnailPreloadProgress.previewsCompleted}/${thumbnailPreloadProgress.previewsTotal} previews`
          : "Manual thumb + preview fill"}
      </div>
    </div>
  );
}
