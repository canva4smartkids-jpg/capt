import { Badge, Button, Card, Dialog, EmptyState, Icon, IconButton, ProgressBar, SegmentedControl, Select, Spinner, TextInput, useToast } from "@captureone/design-system";
import type { MediaMetadata, StorageUsage } from "@captureone/capture-contracts";
import {
  DEFAULT_LIBRARY_QUERY,
  audioSummary,
  bitrateLabel,
  displayTitle,
  durationLabel,
  formatLabel,
  formatResolution,
  isTvCapture,
  queryLibrary,
  sizeLabel,
  storageRatio,
  validationBadge,
  type LibraryFilter,
  type LibraryQuery,
  type LibrarySort,
} from "@captureone/media-metadata";
import { formatBytes } from "@captureone/settings";
import { save } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { assetUrl, thumbnailUrl } from "../lib/assets";
import { formatDate, relativeTime } from "../lib/format";
import { asFailure, invoke } from "../lib/ipc";
import { deleteItem, exportItem, renameItem, revealItem, validateItem } from "../lib/library-actions";
import { navigate } from "../lib/router";
import { describeError, useApp } from "../state/app-state";

type View = "grid" | "list";

const SORTS: { value: LibrarySort; label: string }[] = [
  { value: "newest", label: "Newest first" },
  { value: "oldest", label: "Oldest first" },
  { value: "largest", label: "Largest first" },
  { value: "longest", label: "Longest first" },
  { value: "name", label: "Name" },
];

function loadView(): View {
  try {
    return localStorage.getItem("library.view") === "list" ? "list" : "grid";
  } catch {
    return "grid";
  }
}

export function LibraryPage() {
  const app = useApp();
  const toast = useToast();
  const [items, setItems] = useState<MediaMetadata[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [storage, setStorage] = useState<StorageUsage | null>(null);
  const [query, setQuery] = useState<LibraryQuery>(DEFAULT_LIBRARY_QUERY);
  const [view, setView] = useState<View>(loadView);
  const [preview, setPreview] = useState<MediaMetadata | null>(null);
  const [toDelete, setToDelete] = useState<MediaMetadata | null>(null);
  const [deleteFile, setDeleteFile] = useState(true);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [busyIds, setBusyIds] = useState<Set<string>>(new Set());
  const [recovering, setRecovering] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const [list, usage] = await Promise.all([invoke("library_list"), invoke("library_storage").catch(() => null)]);
      setItems(list);
      setStorage(usage);
      setLoadError(null);
    } catch (e) {
      setLoadError(describeError(asFailure(e).toJSON()));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh, app.libraryVersion]);

  useEffect(() => {
    try {
      localStorage.setItem("library.view", view);
    } catch {
      /* storage unavailable */
    }
  }, [view]);

  // "/" focuses search.
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (e.key === "/" && !(t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable))) {
        e.preventDefault();
        searchRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const visible = useMemo(() => queryLibrary(items, query), [items, query]);
  const counts = useMemo(() => ({ all: items.length, video: items.filter((i) => i.kind === "video").length, screenshot: items.filter((i) => i.kind === "screenshot").length }), [items]);

  const withBusy = async (id: string, fn: () => Promise<void>) => {
    setBusyIds((s) => new Set(s).add(id));
    try {
      await fn();
    } finally {
      setBusyIds((s) => {
        const n = new Set(s);
        n.delete(id);
        return n;
      });
    }
  };

  const doRename = (m: MediaMetadata, title: string, renameFile: boolean) =>
    withBusy(m.id, async () => {
      try {
        const next = await renameItem(items, m.id, title, renameFile);
        setItems(next);
        setRenaming(null);
        setPreview((p) => (p?.id === m.id ? (next.find((x) => x.id === m.id) ?? p) : p));
      } catch (e) {
        app.notifyError("Rename failed", e);
      }
    });

  const doValidate = (m: MediaMetadata) =>
    withBusy(m.id, async () => {
      try {
        const { items: next, item } = await validateItem(items, m.id);
        setItems(next);
        setPreview((p) => (p?.id === m.id ? item : p));
        const b = validationBadge(item);
        toast.push({ tone: b.tone === "success" ? "success" : b.tone === "danger" ? "danger" : "warning", title: `${b.label}: ${displayTitle(item)}`, message: b.detail });
      } catch (e) {
        app.notifyError("Validation failed", e);
      }
    });

  const doExport = (m: MediaMetadata) =>
    withBusy(m.id, async () => {
      try {
        const path = await exportItem(m, (opts) => save(opts));
        if (path) toast.push({ tone: "success", title: "Exported", message: path });
      } catch (e) {
        app.notifyError("Export failed", e);
      }
    });

  const doReveal = (m: MediaMetadata) => revealItem(m.id).catch((e) => app.notifyError("Could not open the folder", e));

  const confirmDelete = async () => {
    const m = toDelete;
    if (!m) return;
    setToDelete(null);
    await withBusy(m.id, async () => {
      try {
        setItems(await deleteItem(items, m.id, deleteFile));
        if (preview?.id === m.id) setPreview(null);
        toast.push({ tone: "success", title: deleteFile ? "Deleted" : "Removed from library", message: displayTitle(m) });
        invoke("library_storage").then(setStorage, () => undefined);
      } catch (e) {
        app.notifyError("Delete failed", e);
      }
    });
  };

  const recover = async () => {
    setRecovering(true);
    try {
      const found = await invoke("recover_recordings");
      toast.push({ tone: found.length ? "success" : "info", title: found.length ? `Recovered ${found.length} recording(s)` : "No interrupted recordings found" });
      if (found.length) await refresh();
    } catch (e) {
      app.notifyError("Recovery failed", e);
    } finally {
      setRecovering(false);
    }
  };

  const actions: ItemActions = {
    open: setPreview,
    rename: (m) => setRenaming(m.id),
    commitRename: (m, t, f) => void doRename(m, t, f),
    cancelRename: () => setRenaming(null),
    export: (m) => void doExport(m),
    reveal: (m) => void doReveal(m),
    validate: (m) => void doValidate(m),
    remove: (m) => {
      setDeleteFile(true);
      setToDelete(m);
    },
  };

  const ratio = storage ? storageRatio(storage.totalBytes, storage.freeDiskBytes) : 0;

  return (
    <div className="page page--library">
      <header className="page-header">
        <div>
          <h1>Library</h1>
          <p className="muted">
            {counts.video} recording{counts.video === 1 ? "" : "s"} · {counts.screenshot} screenshot{counts.screenshot === 1 ? "" : "s"}
          </p>
        </div>
        <div className="page-header__actions">
          <Button variant="ghost" icon="refresh" loading={recovering} onClick={() => void recover()}>
            Recover interrupted
          </Button>
          <IconButton icon="refresh" label="Reload library" onClick={() => void refresh()} />
        </div>
      </header>

      {storage && (
        <Card className="storage-card" padded>
          <div className="storage-card__row">
            <Icon name="hdd" size={20} />
            <div className="storage-card__bar">
              <ProgressBar
                value={ratio}
                tone={storage.freeDiskBytes < 2 * 1024 ** 3 ? "danger" : ratio > 0.5 ? "warning" : "accent"}
                label="Library storage"
                showValue
                valueText={`${formatBytes(storage.totalBytes)} used by CaptureOne · ${formatBytes(storage.freeDiskBytes)} free`}
              />
            </div>
          </div>
          <p className="storage-card__dir" title={storage.outputDirectory}>
            <Icon name="folder" size={13} /> {storage.outputDirectory}
          </p>
        </Card>
      )}

      <div className="library-toolbar" role="toolbar" aria-label="Library filters">
        <SegmentedControl<LibraryFilter>
          label="Filter by type"
          value={query.filter}
          onChange={(filter) => setQuery((q) => ({ ...q, filter }))}
          options={[
            { value: "all", label: `All ${counts.all}` },
            { value: "video", label: `Videos ${counts.video}`, icon: "video" },
            { value: "screenshot", label: `Screenshots ${counts.screenshot}`, icon: "image" },
          ]}
        />
        <TextInput
          ref={searchRef}
          icon="search"
          type="search"
          placeholder="Search  ( / )"
          aria-label="Search library"
          value={query.search}
          onChange={(e) => setQuery((q) => ({ ...q, search: e.target.value }))}
          className="library-toolbar__search"
        />
        <Select<LibrarySort> aria-label="Sort" value={query.sort} options={SORTS} onValueChange={(sort) => setQuery((q) => ({ ...q, sort }))} />
        <div className="library-toolbar__views">
          <IconButton icon="library" label="Grid view" pressed={view === "grid"} onClick={() => setView("grid")} />
          <IconButton icon="list" label="List view" pressed={view === "list"} onClick={() => setView("list")} />
        </div>
      </div>

      {loadError ? (
        <EmptyState icon="alert" title="Could not load the library" description={loadError} action={<Button onClick={() => void refresh()}>Try again</Button>} />
      ) : loading && items.length === 0 ? (
        <div className="center-pad">
          <Spinner size={24} label="Loading library" />
        </div>
      ) : items.length === 0 ? (
        <EmptyState
          icon="video"
          title="No recordings yet"
          description="Recordings and screenshots you capture appear here, validated and ready to export."
          action={
            <Button variant="primary" icon="record" onClick={() => navigate("record")}>
              Start recording
            </Button>
          }
        />
      ) : visible.length === 0 ? (
        <EmptyState icon="search" title="No matches" description="Try a different search or filter." action={<Button onClick={() => setQuery(DEFAULT_LIBRARY_QUERY)}>Clear filters</Button>} />
      ) : view === "grid" ? (
        <ul className="media-grid" aria-label="Recordings">
          {visible.map((m) => (
            <MediaCard key={m.id} m={m} busy={busyIds.has(m.id)} renaming={renaming === m.id} actions={actions} />
          ))}
        </ul>
      ) : (
        <MediaTable items={visible} busyIds={busyIds} renaming={renaming} actions={actions} />
      )}

      <PreviewDialog m={preview} onClose={() => setPreview(null)} actions={actions} busy={preview ? busyIds.has(preview.id) : false} />

      <Dialog
        open={toDelete !== null}
        onClose={() => setToDelete(null)}
        role="alertdialog"
        size="sm"
        title={`Delete “${toDelete ? displayTitle(toDelete) : ""}”?`}
        description={deleteFile ? "The file is permanently deleted from disk." : "The item is removed from the library; the file stays on disk."}
        footer={
          <>
            <Button onClick={() => setToDelete(null)}>Cancel</Button>
            <Button variant="danger" icon="trash" onClick={() => void confirmDelete()}>
              {deleteFile ? "Delete file" : "Remove"}
            </Button>
          </>
        }
      >
        <label className="checkbox">
          <input type="checkbox" checked={deleteFile} onChange={(e) => setDeleteFile(e.target.checked)} />
          Also delete the file from disk
        </label>
      </Dialog>
    </div>
  );
}

interface ItemActions {
  open: (m: MediaMetadata) => void;
  rename: (m: MediaMetadata) => void;
  commitRename: (m: MediaMetadata, title: string, renameFile: boolean) => void;
  cancelRename: () => void;
  export: (m: MediaMetadata) => void;
  reveal: (m: MediaMetadata) => void;
  validate: (m: MediaMetadata) => void;
  remove: (m: MediaMetadata) => void;
}

function InlineRename({ m, onCommit, onCancel }: { m: MediaMetadata; onCommit: (title: string, renameFile: boolean) => void; onCancel: () => void }) {
  const [value, setValue] = useState(displayTitle(m));
  const [renameFile, setRenameFile] = useState(false);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);
  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      onCommit(value, renameFile);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onCancel();
    }
  };
  const invalid = value.trim().length === 0 || value.trim().length > 120;
  return (
    <div className="inline-rename" onClick={(e) => e.stopPropagation()}>
      <TextInput ref={ref} aria-label="New title" value={value} aria-invalid={invalid} maxLength={120} onChange={(e) => setValue(e.target.value)} onKeyDown={onKey} />
      <label className="checkbox checkbox--sm">
        <input type="checkbox" checked={renameFile} onChange={(e) => setRenameFile(e.target.checked)} /> Rename file too
      </label>
      <div className="inline-rename__actions">
        <IconButton icon="check" label="Save title" size="sm" variant="primary" disabled={invalid} onClick={() => onCommit(value, renameFile)} />
        <IconButton icon="close" label="Cancel rename" size="sm" onClick={onCancel} />
      </div>
    </div>
  );
}

function ItemMenu({ m, actions, busy }: { m: MediaMetadata; actions: ItemActions; busy: boolean }) {
  return (
    <div className="item-actions" onClick={(e) => e.stopPropagation()}>
      <IconButton icon="edit" label={`Rename ${displayTitle(m)}`} size="sm" disabled={busy} onClick={() => actions.rename(m)} />
      <IconButton icon="export" label="Export…" size="sm" disabled={busy} onClick={() => actions.export(m)} />
      <IconButton icon="reveal" label="Show in folder" size="sm" onClick={() => actions.reveal(m)} />
      <IconButton icon="refresh" label="Re-validate" size="sm" disabled={busy} onClick={() => actions.validate(m)} />
      <IconButton icon="trash" label={`Delete ${displayTitle(m)}`} size="sm" disabled={busy} onClick={() => actions.remove(m)} />
    </div>
  );
}

function Thumb({ m }: { m: MediaMetadata }) {
  const [failed, setFailed] = useState(false);
  const url = thumbnailUrl(m);
  if (!url || failed) {
    return (
      <div className="media-thumb__placeholder">
        <Icon name={m.kind === "video" ? "video" : "image"} size={30} />
      </div>
    );
  }
  return <img src={url} alt="" loading="lazy" draggable={false} onError={() => setFailed(true)} />;
}

function MediaCard({ m, busy, renaming, actions }: { m: MediaMetadata; busy: boolean; renaming: boolean; actions: ItemActions }) {
  const badge = validationBadge(m);
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      actions.open(m);
    } else if (e.key === "F2") {
      e.preventDefault();
      actions.rename(m);
    } else if (e.key === "Delete") {
      e.preventDefault();
      actions.remove(m);
    }
  };
  return (
    <li className={`media-card ${busy ? "is-busy" : ""}`}>
      <div className="media-card__inner" role="button" tabIndex={0} aria-label={`Open ${displayTitle(m)}`} onClick={() => !renaming && actions.open(m)} onKeyDown={onKey}>
        <div className="media-thumb">
          <Thumb m={m} />
          <span className="media-thumb__kind">
            <Icon name={m.kind === "video" ? "video" : "image"} size={13} /> {formatLabel(m)}
          </span>
          {m.kind === "video" && durationLabel(m) && <span className="media-thumb__duration">{durationLabel(m)}</span>}
          {busy && (
            <span className="media-thumb__busy">
              <Spinner size={20} />
            </span>
          )}
        </div>
        <div className="media-card__body">
          {renaming ? (
            <InlineRename m={m} onCommit={(t, f) => actions.commitRename(m, t, f)} onCancel={actions.cancelRename} />
          ) : (
            <h3 className="media-card__title" title={m.fileName} onDoubleClick={(e) => (e.stopPropagation(), actions.rename(m))}>
              {displayTitle(m)}
            </h3>
          )}
          {isTvCapture(m) && (
            <Badge tone="warning" icon="tv" title="Captured on a TV through an undocumented LG webOS call; protected content may be black.">
              From TV · experimental
            </Badge>
          )}
          <div className="media-card__meta">
            <span>{sizeLabel(m)}</span>
            <span>{formatResolution(m)}</span>
            <span title={formatDate(m.createdAt)}>{relativeTime(m.createdAt)}</span>
          </div>
          <div className="media-card__foot">
            <Badge tone={badge.tone} title={badge.detail} dot>
              {badge.label}
            </Badge>
            <ItemMenu m={m} actions={actions} busy={busy} />
          </div>
        </div>
      </div>
    </li>
  );
}

function MediaTable({ items, busyIds, renaming, actions }: { items: MediaMetadata[]; busyIds: Set<string>; renaming: string | null; actions: ItemActions }) {
  return (
    <div className="table-wrap">
      <table className="media-table">
        <thead>
          <tr>
            <th scope="col">Name</th>
            <th scope="col">Duration</th>
            <th scope="col">Size</th>
            <th scope="col">Resolution</th>
            <th scope="col">Format</th>
            <th scope="col">Created</th>
            <th scope="col">Check</th>
            <th scope="col">
              <span className="co-sr-only">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {items.map((m) => {
            const b = validationBadge(m);
            return (
              <tr key={m.id} className={busyIds.has(m.id) ? "is-busy" : undefined}>
                <td>
                  {renaming === m.id ? (
                    <InlineRename m={m} onCommit={(t, f) => actions.commitRename(m, t, f)} onCancel={actions.cancelRename} />
                  ) : (
                    <button type="button" className="link-btn media-table__name" onClick={() => actions.open(m)} title={m.fileName}>
                      <span className="media-table__thumb">
                        <Thumb m={m} />
                      </span>
                      {displayTitle(m)}
                    </button>
                  )}
                </td>
                <td>{durationLabel(m) ?? "—"}</td>
                <td>{sizeLabel(m)}</td>
                <td>{formatResolution(m)}</td>
                <td>{formatLabel(m)}</td>
                <td title={formatDate(m.createdAt)}>{relativeTime(m.createdAt)}</td>
                <td>
                  <Badge tone={b.tone} title={b.detail} dot>
                    {b.label}
                  </Badge>
                </td>
                <td>
                  <ItemMenu m={m} actions={actions} busy={busyIds.has(m.id)} />
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

/** Video preview source: loopback stream where the asset protocol cannot stream media (Linux), else asset URL. */
function useVideoSrc(m: MediaMetadata | null): string | undefined | null {
  const [src, setSrc] = useState<string | undefined | null>(null);
  useEffect(() => {
    if (!m) return;
    if (m.kind !== "video") {
      setSrc(assetUrl(m.path));
      return;
    }
    let live = true;
    setSrc(null);
    invoke("media_stream_url", { id: m.id })
      .then((url) => live && setSrc(url ?? assetUrl(m.path)))
      .catch(() => live && setSrc(assetUrl(m.path)));
    return () => {
      live = false;
    };
  }, [m]);
  return src;
}

function PreviewDialog({ m, onClose, actions, busy }: { m: MediaMetadata | null; onClose: () => void; actions: ItemActions; busy: boolean }) {
  const [mediaError, setMediaError] = useState(false);
  useEffect(() => setMediaError(false), [m?.id]);
  const streamSrc = useVideoSrc(m);
  if (!m) return null;
  const badge = validationBadge(m);
  const src = streamSrc ?? undefined;
  return (
    <Dialog
      open
      onClose={onClose}
      size="xl"
      title={displayTitle(m)}
      description={m.path}
      footer={
        <>
          <Button icon="reveal" variant="ghost" onClick={() => actions.reveal(m)}>
            Show in folder
          </Button>
          <Button icon="refresh" variant="ghost" loading={busy} onClick={() => actions.validate(m)}>
            Re-validate
          </Button>
          <Button icon="export" onClick={() => actions.export(m)}>
            Export…
          </Button>
          <Button icon="trash" variant="danger" onClick={() => actions.remove(m)}>
            Delete
          </Button>
        </>
      }
    >
      <div className="preview">
        <div className="preview__media">
          {streamSrc === null ? (
            <Spinner size={22} label="Loading preview" />
          ) : mediaError || !src ? (
            <EmptyState icon="alert" title="Preview unavailable" description="The file could not be loaded. It may have been moved or deleted, or the system lacks a decoder for it." />
          ) : m.kind === "video" ? (
            <video key={m.id} src={src} controls preload="metadata" onError={() => setMediaError(true)} aria-label={`Video preview of ${displayTitle(m)}`} />
          ) : (
            <img key={m.id} src={src} alt={`Screenshot ${displayTitle(m)}`} onError={() => setMediaError(true)} />
          )}
        </div>
        <dl className="preview__meta">
          <div>
            <dt>Validation</dt>
            <dd>
              <Badge tone={badge.tone} dot>
                {badge.label}
              </Badge>
              <p className="muted small">{badge.detail}</p>
              {m.validation?.issues.map((i) => (
                <p key={i.code + i.message} className={i.severity === "error" ? "text-danger small" : "text-warning small"}>
                  {i.code}: {i.message}
                </p>
              ))}
            </dd>
          </div>
          <div>
            <dt>Created</dt>
            <dd>{formatDate(m.createdAt)}</dd>
          </div>
          <div>
            <dt>Size</dt>
            <dd>{sizeLabel(m)}</dd>
          </div>
          <div>
            <dt>Resolution</dt>
            <dd>{formatResolution(m)}</dd>
          </div>
          {m.kind === "video" && (
            <>
              <div>
                <dt>Duration</dt>
                <dd>{durationLabel(m) ?? "—"}</dd>
              </div>
              <div>
                <dt>Video</dt>
                <dd>
                  {formatLabel(m)}
                  {m.fps ? ` · ${Math.round(m.fps * 100) / 100} fps` : ""}
                  {bitrateLabel(m) ? ` · ${bitrateLabel(m)}` : ""}
                </dd>
              </div>
              <div>
                <dt>Audio</dt>
                <dd>{audioSummary(m)}</dd>
              </div>
            </>
          )}
          {m.kind === "screenshot" && (
            <div>
              <dt>Format</dt>
              <dd>{formatLabel(m)}</dd>
            </div>
          )}
          {m.sourceName && (
            <div>
              <dt>Source</dt>
              <dd>
                {m.sourceName}
                {isTvCapture(m) && <p className="muted small">Experimental TV screenshot (undocumented LG webOS API). DRM-protected content is blanked by the TV.</p>}
              </dd>
            </div>
          )}
          <div>
            <dt>File</dt>
            <dd className="break">{m.fileName}</dd>
          </div>
        </dl>
      </div>
    </Dialog>
  );
}
