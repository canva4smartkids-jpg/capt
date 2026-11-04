import { Button } from "@captureone/design-system";
import { useEffect, useRef } from "react";

export function CountdownOverlay({ remaining, onCancel }: { remaining: number; onCancel: () => void }) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    cancelRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onCancel();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);
  return (
    <div className="countdown-overlay" role="alertdialog" aria-modal="true" aria-labelledby="countdown-title" aria-describedby="countdown-number">
      <p id="countdown-title" className="countdown-overlay__title">
        Recording starts in
      </p>
      <div className="countdown-overlay__ring" key={remaining}>
        <span id="countdown-number" className="countdown-overlay__number" aria-live="assertive">
          {remaining}
        </span>
      </div>
      <Button ref={cancelRef} variant="secondary" size="lg" icon="close" onClick={onCancel}>
        Cancel (Esc)
      </Button>
    </div>
  );
}
