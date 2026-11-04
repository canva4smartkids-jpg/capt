import { useEffect, useState } from "react";

export type MainRoute = "record" | "library" | "settings" | "devices";
export type Route = MainRoute | "indicator" | "region";

export interface ParsedRoute {
  route: Route;
  params: URLSearchParams;
}

const ROUTES: Record<string, Route> = {
  "": "record",
  record: "record",
  library: "library",
  settings: "settings",
  devices: "devices",
  indicator: "indicator",
  region: "region",
};

/** Parses "#/library?x=1" style hashes. Unknown paths fall back to the Record page. */
export function parseHash(hash: string): ParsedRoute {
  const raw = hash.replace(/^#\/?/, "");
  const [path = "", query = ""] = raw.split("?", 2);
  const route = ROUTES[path.replace(/\/+$/, "")] ?? "record";
  return { route, params: new URLSearchParams(query) };
}

export function hrefFor(route: MainRoute): string {
  return route === "record" ? "#/" : `#/${route}`;
}

export function navigate(route: MainRoute): void {
  window.location.hash = hrefFor(route);
}

export function useHashRoute(): ParsedRoute {
  const [parsed, setParsed] = useState(() => parseHash(window.location.hash));
  useEffect(() => {
    const onChange = () => setParsed(parseHash(window.location.hash));
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
  }, []);
  return parsed;
}
