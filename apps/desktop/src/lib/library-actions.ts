/**
 * Library operations used by the Library page. Kept free of React so they can be
 * unit-tested with a mocked IPC layer. Each returns the next item list (or a
 * cancelled marker for user-dismissed dialogs).
 */
import type { MediaMetadata } from "@captureone/capture-contracts";
import { exportFileName, fileExtension, removeMedia, upsertMedia, validateTitle } from "@captureone/media-metadata";
import { CaptureFailure } from "@captureone/capture-contracts";
import { invoke } from "./ipc";

export type SaveDialog = (opts: { defaultPath: string; filters: { name: string; extensions: string[] }[]; title: string }) => Promise<string | null>;

export async function renameItem(items: readonly MediaMetadata[], id: string, title: string, renameFile = false): Promise<MediaMetadata[]> {
  const problem = validateTitle(title);
  if (problem) throw new CaptureFailure({ code: "INVALID_CONFIG", message: problem, recoverable: true });
  const updated = await invoke("library_rename", renameFile ? { id, title: title.trim(), renameFile } : { id, title: title.trim() });
  return upsertMedia(items, updated);
}

export async function deleteItem(items: readonly MediaMetadata[], id: string, deleteFile: boolean): Promise<MediaMetadata[]> {
  await invoke("library_delete", { id, deleteFile });
  return removeMedia(items, id);
}

export async function validateItem(items: readonly MediaMetadata[], id: string): Promise<{ items: MediaMetadata[]; item: MediaMetadata }> {
  const item = await invoke("library_validate", { id });
  return { items: upsertMedia(items, item), item };
}

export function exportFilters(m: MediaMetadata): { name: string; extensions: string[] }[] {
  const ext = fileExtension(m.fileName) || m.container;
  const name = m.kind === "video" ? "MP4 video" : ext === "png" ? "PNG image" : "JPEG image";
  return [{ name, extensions: ext === "jpeg" || ext === "jpg" ? ["jpg", "jpeg"] : [ext] }];
}

/** Opens the save dialog and copies the file. Returns the written path, or null if the user cancelled. */
export async function exportItem(m: MediaMetadata, save: SaveDialog): Promise<string | null> {
  const destination = await save({ defaultPath: exportFileName(m), filters: exportFilters(m), title: "Export recording" });
  if (!destination) return null;
  return invoke("library_export", { id: m.id, destination });
}

export async function revealItem(id: string): Promise<void> {
  await invoke("library_reveal", { id });
}
