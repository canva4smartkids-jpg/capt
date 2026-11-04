import { useEffect, useRef, useState } from "react";
import type { CaptureSource } from "@captureone/capture-contracts";
import { invoke } from "./ipc";

export type ThumbState = { status: "loading" } | { status: "ok"; url: string } | { status: "error"; message: string };

/** Lazily fetches source thumbnails (2 at a time), re-fetching when `version` changes. */
export function useThumbnails(sources: readonly CaptureSource[], maxWidth: number, version: number): Record<string, ThumbState> {
  const [thumbs, setThumbs] = useState<Record<string, ThumbState>>({});
  const key = sources.map((s) => `${s.kind}:${s.id}`).join("|");
  const sourcesRef = useRef(sources);
  sourcesRef.current = sources;

  useEffect(() => {
    let cancelled = false;
    const queue = [...sourcesRef.current];
    setThumbs(Object.fromEntries(queue.map((s) => [s.id, { status: "loading" } as ThumbState])));
    const worker = async () => {
      for (let s = queue.shift(); s && !cancelled; s = queue.shift()) {
        const src = s;
        try {
          const url = await invoke("get_source_thumbnail", { kind: src.kind, sourceId: src.id, maxWidth });
          if (!cancelled) setThumbs((t) => ({ ...t, [src.id]: { status: "ok", url } }));
        } catch (e) {
          if (!cancelled) setThumbs((t) => ({ ...t, [src.id]: { status: "error", message: e instanceof Error ? e.message : String(e) } }));
        }
      }
    };
    void Promise.all([worker(), worker()]);
    return () => {
      cancelled = true;
    };
  }, [key, maxWidth, version]);

  return thumbs;
}
