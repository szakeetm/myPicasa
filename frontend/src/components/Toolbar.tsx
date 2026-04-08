type ToolbarProps = {
  query: string;
  mediaKind: string;
  dateFrom: string;
  dateTo: string;
  onQueryChange: (value: string) => void;
  onMediaKindChange: (value: string) => void;
  onDateFromChange: (value: string) => void;
  onDateToChange: (value: string) => void;
  onApply: () => void;
};

export function Toolbar({
  query,
  mediaKind,
  dateFrom,
  dateTo,
  onQueryChange,
  onMediaKindChange,
  onDateFromChange,
  onDateToChange,
  onApply,
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
      <input type="date" value={dateFrom} onChange={(event) => onDateFromChange(event.target.value)} />
      <div className="button-row">
        <input type="date" value={dateTo} onChange={(event) => onDateToChange(event.target.value)} />
        <button className="button-secondary" onClick={onApply}>
          Apply
        </button>
      </div>
    </div>
  );
}
