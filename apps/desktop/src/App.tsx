import { lazy, Suspense, useEffect } from "react";
import { ToastProvider, matchesAccelerator } from "@captureone/design-system";
import { navigate, useHashRoute, type MainRoute } from "./lib/router";
import { AppProvider } from "./state/app-state";
import { Sidebar } from "./components/Sidebar";
import { RecordPage } from "./pages/RecordPage";
import { IndicatorView } from "./pages/IndicatorView";
import { RegionView } from "./pages/RegionView";

const LibraryPage = lazy(() => import("./pages/LibraryPage").then((m) => ({ default: m.LibraryPage })));
const SettingsPage = lazy(() => import("./pages/SettingsPage").then((m) => ({ default: m.SettingsPage })));
const DevicesPage = lazy(() => import("./pages/DevicesPage").then((m) => ({ default: m.DevicesPage })));

const NAV_KEYS: Record<string, MainRoute> = { "1": "record", "2": "library", "3": "settings", "4": "devices" };

export function App() {
  const { route, params } = useHashRoute();
  if (route === "indicator") return <IndicatorView />;
  if (route === "region") return <RegionView params={params} />;
  return (
    <ToastProvider>
      <AppProvider>
        <MainWindow route={route} />
      </AppProvider>
    </ToastProvider>
  );
}

function MainWindow({ route }: { route: MainRoute }) {
  // In-app navigation shortcuts: Ctrl/Cmd+1..4.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      for (const [digit, target] of Object.entries(NAV_KEYS)) {
        if (matchesAccelerator(e, `CommandOrControl+${digit}`)) {
          e.preventDefault();
          navigate(target);
          return;
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    const titles: Record<MainRoute, string> = { record: "Record", library: "Library", settings: "Settings", devices: "Devices" };
    document.title = `${titles[route]} · CaptureOne`;
    document.getElementById("main-content")?.focus({ preventScroll: true });
  }, [route]);

  return (
    <div className="app-shell">
      <a className="skip-link" href="#main-content">
        Skip to content
      </a>
      <Sidebar route={route} />
      <main id="main-content" className="app-main" tabIndex={-1}>
        <Suspense fallback={<div className="page-loading" aria-busy="true" />}>
          {route === "record" && <RecordPage />}
          {route === "library" && <LibraryPage />}
          {route === "settings" && <SettingsPage />}
          {route === "devices" && <DevicesPage />}
        </Suspense>
      </main>
    </div>
  );
}
