import "./styles.css";
import { App } from "./app.js";
import { createAdapter, currentEnv, detectPlatform } from "./platform/detect.js";
import { createConnect } from "./screens/connect.js";
import { createHome } from "./screens/home.js";
import { createHowto } from "./screens/howto.js";
import { createRecordings } from "./screens/recordings.js";
import { createRemote } from "./screens/remote.js";

function boot(): void {
  const root = document.getElementById("app");
  if (!root) return;
  const platform = detectPlatform(currentEnv());
  document.documentElement.setAttribute("data-platform", platform);
  const app = new App(root, createAdapter(platform), {
    home: createHome,
    connect: createConnect,
    recordings: createRecordings,
    remote: createRemote,
    howto: createHowto,
  });
  (window as unknown as { captureOneTv?: App }).captureOneTv = app;
  app.start().catch((e: unknown) => console.error("CaptureOne TV failed to start", e));
}

if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", boot);
else boot();
