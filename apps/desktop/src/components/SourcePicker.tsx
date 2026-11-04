import { Badge, Button, EmptyState, Icon, Spinner, TabPanel, Tabs, type TabItem } from "@captureone/design-system";
import type { CaptureSource } from "@captureone/capture-contracts";
import { useState } from "react";
import type { Gate, RecordForm, SourceTab } from "../lib/record-form";
import { useThumbnails, type ThumbState } from "../lib/use-thumbnails";

interface Props {
  form: RecordForm;
  onChange: (patch: Partial<RecordForm>) => void;
  displays: CaptureSource[];
  windows: CaptureSource[];
  loading: boolean;
  error: string | null;
  windowGate: Gate;
  regionGate: Gate;
  onSelectRegion: (displayId: string) => void;
  selectingRegion: boolean;
  thumbVersion: number;
  disabled: boolean;
}

export function SourcePicker({ form, onChange, displays, windows, loading, error, windowGate, regionGate, onSelectRegion, selectingRegion, thumbVersion, disabled }: Props) {
  const displayThumbs = useThumbnails(displays, 360, thumbVersion);
  const windowThumbs = useThumbnails(form.sourceTab === "window" ? windows : [], 320, thumbVersion);
  const [regionDisplay, setRegionDisplay] = useState<string | null>(null);
  const regionDisplayId = regionDisplay ?? form.region?.displayId ?? displays.find((d) => d.isPrimary)?.id ?? displays[0]?.id ?? null;

  const tabs: TabItem<SourceTab>[] = [
    { id: "display", label: "Displays", icon: "monitor", badge: displays.length || undefined },
    { id: "window", label: "Windows", icon: "window", disabled: !windowGate.allowed, badge: windowGate.allowed ? windows.length || undefined : undefined },
    { id: "region", label: "Region", icon: "region", disabled: !regionGate.allowed },
  ];

  return (
    <div className="source-picker">
      <Tabs idPrefix="source" label="Capture source type" tabs={tabs} value={form.sourceTab} onChange={(sourceTab) => onChange({ sourceTab })} />
      {error && (
        <div className="inline-alert inline-alert--danger" role="alert">
          <Icon name="alert" size={16} /> {error}
        </div>
      )}
      <TabPanel idPrefix="source" id="display" active={form.sourceTab === "display"}>
        {loading && displays.length === 0 ? (
          <div className="source-grid">
            <div className="source-card is-skeleton" />
            <div className="source-card is-skeleton" />
          </div>
        ) : displays.length === 0 ? (
          <EmptyState icon="monitor" title="No displays found" description="CaptureOne could not enumerate any display on this system." />
        ) : (
          <div className="source-grid" role="radiogroup" aria-label="Display to record">
            {displays.map((d) => (
              <SourceCard key={d.id} source={d} thumb={displayThumbs[d.id]} selected={form.displayId === d.id} disabled={disabled} onSelect={() => onChange({ displayId: d.id })} />
            ))}
          </div>
        )}
      </TabPanel>
      <TabPanel idPrefix="source" id="window" active={form.sourceTab === "window"}>
        {windows.length === 0 ? (
          <EmptyState icon="window" title={loading ? "Looking for windows…" : "No capturable windows"} description="Open the window you want to record, then refresh the list." />
        ) : (
          <div className="source-grid source-grid--windows" role="radiogroup" aria-label="Window to record">
            {windows.map((w) => (
              <SourceCard key={w.id} source={w} thumb={windowThumbs[w.id]} selected={form.windowId === w.id} disabled={disabled} onSelect={() => onChange({ windowId: w.id })} />
            ))}
          </div>
        )}
        {windowGate.reason && <p className="hint">{windowGate.reason}</p>}
      </TabPanel>
      <TabPanel idPrefix="source" id="region" active={form.sourceTab === "region"}>
        <div className="region-panel">
          <div className="region-panel__displays" role="radiogroup" aria-label="Display for region selection">
            {displays.map((d) => (
              <button
                key={d.id}
                type="button"
                role="radio"
                aria-checked={regionDisplayId === d.id}
                className={`chip ${regionDisplayId === d.id ? "is-selected" : ""}`}
                onClick={() => setRegionDisplay(d.id)}
                disabled={disabled}
              >
                <Icon name="monitor" size={15} /> {d.name} <span className="chip__meta">{d.width}×{d.height}</span>
              </button>
            ))}
          </div>
          <div className="region-panel__body">
            {form.region ? (
              <div className="region-summary">
                <Icon name="region" size={22} />
                <div>
                  <div className="region-summary__size">
                    {form.region.rect.width} × {form.region.rect.height} px
                  </div>
                  <div className="region-summary__pos">
                    at ({form.region.rect.x}, {form.region.rect.y}) on {displays.find((d) => d.id === form.region?.displayId)?.name ?? "display"}
                  </div>
                </div>
              </div>
            ) : (
              <p className="muted">Drag over the screen to choose the area to capture. Press Enter to confirm or Esc to cancel.</p>
            )}
            <Button
              variant={form.region ? "secondary" : "primary"}
              icon="region"
              loading={selectingRegion}
              disabled={!regionDisplayId || disabled}
              onClick={() => regionDisplayId && onSelectRegion(regionDisplayId)}
            >
              {form.region ? "Reselect region" : "Select region…"}
            </Button>
          </div>
        </div>
      </TabPanel>
    </div>
  );
}

function SourceCard({ source, thumb, selected, disabled, onSelect }: { source: CaptureSource; thumb: ThumbState | undefined; selected: boolean; disabled: boolean; onSelect: () => void }) {
  return (
    <button type="button" role="radio" aria-checked={selected} className={`source-card ${selected ? "is-selected" : ""}`} onClick={onSelect} disabled={disabled}>
      <div className="source-card__thumb">
        {thumb?.status === "ok" ? (
          <img src={thumb.url} alt="" draggable={false} />
        ) : thumb?.status === "error" ? (
          <div className="source-card__placeholder" title={thumb.message}>
            <Icon name={source.kind === "window" ? "window" : "monitor"} size={28} />
            <span>Preview unavailable</span>
          </div>
        ) : (
          <div className="source-card__placeholder">
            <Spinner size={18} />
          </div>
        )}
        {selected && (
          <span className="source-card__check" aria-hidden>
            <Icon name="check" size={14} />
          </span>
        )}
      </div>
      <div className="source-card__meta">
        <span className="source-card__name" title={source.name}>
          {source.name || (source.kind === "window" ? "Untitled window" : "Display")}
        </span>
        <span className="source-card__sub">
          {source.appName ? `${source.appName} · ` : ""}
          {source.width}×{source.height}
          {source.scaleFactor && source.scaleFactor !== 1 ? ` · ${source.scaleFactor}×` : ""}
        </span>
      </div>
      {source.isPrimary && (
        <Badge tone="accent" className="source-card__badge">
          Primary
        </Badge>
      )}
    </button>
  );
}
