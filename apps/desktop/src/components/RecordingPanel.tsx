import { Badge, Button, Dialog, Icon, IconButton } from "@captureone/design-system";
import { formatBytes, formatDuration } from "@captureone/settings";
import { displayTitle, durationLabel, formatResolution, sizeLabel, validationBadge } from "@captureone/media-metadata";
import { useEffect, useState } from "react";
import { canCancel, canPause, canResume, canStop, displayElapsedMs, isFinishing, phaseLabel, type RecordingUiState } from "../lib/recording-machine";
import { invoke } from "../lib/ipc";
import { navigate } from "../lib/router";

interface Props {
  state: RecordingUiState;
  pauseSupported: boolean;
  onAction: (a: "pause" | "resume" | "stop" | "cancel") => void;
  onDismiss: () => void;
}

export function RecordingPanel({ state, pauseSupported, onAction, onDismiss }: Props) {
  const [now, setNow] = useState(() => performance.now());
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  useEffect(() => {
    if (state.phase !== "recording") return;
    const id = setInterval(() => setNow(performance.now()), 250);
    return () => clearInterval(id);
  }, [state.phase]);

  if (state.phase === "completed" && state.lastMedia) {
    const m = state.lastMedia;
    const badge = validationBadge(m);
    return (
      <div className="rec-panel rec-panel--done">
        <div className="rec-panel__head">
          <span className="rec-panel__icon rec-panel__icon--ok">
            <Icon name="check" size={20} />
          </span>
          <div>
            <h3>Recording saved</h3>
            <p className="muted" title={m.path}>
              {displayTitle(m)}
            </p>
          </div>
        </div>
        <dl className="stat-list">
          <div>
            <dt>Duration</dt>
            <dd>{durationLabel(m) ?? "—"}</dd>
          </div>
          <div>
            <dt>Size</dt>
            <dd>{sizeLabel(m)}</dd>
          </div>
          <div>
            <dt>Resolution</dt>
            <dd>{formatResolution(m)}</dd>
          </div>
          <div>
            <dt>Check</dt>
            <dd>
              <Badge tone={badge.tone} title={badge.detail}>
                {badge.label}
              </Badge>
            </dd>
          </div>
        </dl>
        <div className="rec-panel__actions">
          <Button icon="library" onClick={() => navigate("library")}>
            Open library
          </Button>
          <Button icon="reveal" variant="ghost" onClick={() => void invoke("library_reveal", { id: m.id }).catch(() => undefined)}>
            Show in folder
          </Button>
          <Button variant="primary" icon="record" onClick={onDismiss}>
            New recording
          </Button>
        </div>
      </div>
    );
  }

  if (state.phase === "failed") {
    return (
      <div className="rec-panel rec-panel--failed" role="alert">
        <div className="rec-panel__head">
          <span className="rec-panel__icon rec-panel__icon--bad">
            <Icon name="alert" size={20} />
          </span>
          <div>
            <h3>Recording failed</h3>
            <p className="muted">{state.error?.message ?? "The recording could not be completed."}</p>
          </div>
        </div>
        <div className="rec-panel__actions">
          <Button variant="primary" onClick={onDismiss}>
            Back
          </Button>
        </div>
      </div>
    );
  }

  const st = state.status;
  const elapsed = displayElapsedMs(state, now);
  const finishing = isFinishing(state);
  return (
    <div className={`rec-panel rec-panel--${state.phase}`} aria-label="Recording controls">
      <div className="rec-panel__status">
        <span className="live-dot" aria-hidden />
        <span>{phaseLabel(state.phase)}</span>
        {st?.encoder && <Badge tone="neutral">{st.encoder}</Badge>}
      </div>
      <div className="rec-panel__timer" role="timer" aria-live="off" aria-label={`Elapsed ${formatDuration(elapsed)}`}>
        {formatDuration(elapsed)}
      </div>
      <dl className="stat-list stat-list--compact">
        <div>
          <dt>Written</dt>
          <dd>{st ? formatBytes(st.bytesWritten) : "—"}</dd>
        </div>
        <div>
          <dt>Frames</dt>
          <dd>{st ? st.framesEncoded.toLocaleString() : "—"}</dd>
        </div>
        <div>
          <dt>Dropped</dt>
          <dd className={st && st.framesDropped > 0 ? "text-warning" : undefined}>{st ? st.framesDropped.toLocaleString() : "—"}</dd>
        </div>
        <div>
          <dt>Free disk</dt>
          <dd>{st?.freeDiskBytes !== undefined ? formatBytes(st.freeDiskBytes) : "—"}</dd>
        </div>
      </dl>
      {state.warnings.length > 0 && (
        <ul className="rec-panel__warnings">
          {state.warnings.map((w) => (
            <li key={w}>
              <Icon name="alert" size={14} /> {w}
            </li>
          ))}
        </ul>
      )}
      <div className="rec-panel__controls">
        {state.phase === "paused" ? (
          <IconButton icon="play" label="Resume recording" variant="secondary" size="lg" disabled={!canResume(state)} onClick={() => onAction("resume")} />
        ) : (
          <IconButton
            icon="pause"
            label={pauseSupported ? "Pause recording" : "Pause is not supported on this platform"}
            variant="secondary"
            size="lg"
            disabled={!pauseSupported || !canPause(state)}
            onClick={() => onAction("pause")}
          />
        )}
        <Button variant="record" size="lg" icon="stop" loading={finishing} disabled={!canStop(state)} onClick={() => onAction("stop")}>
          {finishing ? "Finalizing…" : "Stop"}
        </Button>
        <IconButton icon="trash" label="Discard recording" variant="ghost" size="lg" disabled={!canCancel(state)} onClick={() => setConfirmDiscard(true)} />
      </div>
      <Dialog
        open={confirmDiscard}
        onClose={() => setConfirmDiscard(false)}
        role="alertdialog"
        size="sm"
        title="Discard this recording?"
        description="Recording stops and the partial file is deleted. This cannot be undone."
        footer={
          <>
            <Button onClick={() => setConfirmDiscard(false)}>Keep recording</Button>
            <Button
              variant="danger"
              icon="trash"
              onClick={() => {
                setConfirmDiscard(false);
                onAction("cancel");
              }}
            >
              Discard
            </Button>
          </>
        }
      />
    </div>
  );
}
