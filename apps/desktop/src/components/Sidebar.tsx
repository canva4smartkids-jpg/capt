import { Icon, KeyboardShortcut, type IconName } from "@captureone/design-system";
import { formatDuration } from "@captureone/settings";
import { useEffect, useState } from "react";
import { hrefFor, type MainRoute } from "../lib/router";
import { displayElapsedMs, isBusy, phaseLabel } from "../lib/recording-machine";
import { useApp } from "../state/app-state";

const ITEMS: { route: MainRoute; label: string; icon: IconName; key: string }[] = [
  { route: "record", label: "Record", icon: "record", key: "1" },
  { route: "library", label: "Library", icon: "library", key: "2" },
  { route: "settings", label: "Settings", icon: "settings", key: "3" },
  { route: "devices", label: "Devices", icon: "devices", key: "4" },
];

export function Sidebar({ route }: { route: MainRoute }) {
  const { recording, capabilities } = useApp();
  const busy = isBusy(recording);
  const [now, setNow] = useState(() => performance.now());
  useEffect(() => {
    if (recording.phase !== "recording") return;
    const id = setInterval(() => setNow(performance.now()), 500);
    return () => clearInterval(id);
  }, [recording.phase]);

  return (
    <nav className="sidebar" aria-label="Main">
      <div className="sidebar__brand">
        <img src="/favicon.svg" alt="" width={30} height={30} />
        <div>
          <div className="sidebar__name">CaptureOne</div>
          <div className="sidebar__platform">{capabilities ? `${capabilities.osName} ${capabilities.osVersion}` : "Desktop"}</div>
        </div>
      </div>
      <ul className="sidebar__nav">
        {ITEMS.map((item) => (
          <li key={item.route}>
            <a href={hrefFor(item.route)} className={route === item.route ? "is-active" : undefined} aria-current={route === item.route ? "page" : undefined}>
              <Icon name={item.icon} size={18} />
              <span>{item.label}</span>
              <KeyboardShortcut accelerator={`CommandOrControl+${item.key}`} className="sidebar__kbd" />
            </a>
          </li>
        ))}
      </ul>
      <div className="sidebar__spacer" />
      {busy && (
        <a href={hrefFor("record")} className={`sidebar__live sidebar__live--${recording.phase}`} aria-live="polite">
          <span className="live-dot" aria-hidden />
          <span className="sidebar__live-label">{phaseLabel(recording.phase)}</span>
          {(recording.phase === "recording" || recording.phase === "paused") && <span className="sidebar__live-time">{formatDuration(displayElapsedMs(recording, now))}</span>}
          {recording.phase === "countdown" && recording.countdownRemaining !== null && <span className="sidebar__live-time">{recording.countdownRemaining}</span>}
        </a>
      )}
    </nav>
  );
}
