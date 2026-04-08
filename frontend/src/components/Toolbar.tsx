type ToolbarProps = {
  query: string;
  mediaKind: string;
  timelineLabel?: string;
  onQueryChange: (value: string) => void;
  onMediaKindChange: (value: string) => void;
};

export function Toolbar({
  query,
  mediaKind,
  timelineLabel,
  onQueryChange,
  onMediaKindChange,
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
    </div>
  );
}
