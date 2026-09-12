// A grab bar that reports how far it has been dragged.
//
// Pointer events rather than mouse events, so a pen or a touch drag works the
// same way, and `setPointerCapture` so a fast drag that outruns the handle
// keeps resizing instead of stopping the moment the cursor leaves it. That is
// the difference between a divider that feels attached to the cursor and one
// that keeps slipping.

import { useRef } from "react";

interface Props {
  /** Which axis the drag reads: a column divider is vertical, a row's is not. */
  orientation?: "vertical" | "horizontal";
  /** Pixels moved since the drag began, along the handle's axis. */
  onDrag: (delta: number) => void;
  /** The drag finished; the caller persists whatever it settled on. */
  onEnd: () => void;
  /** Restores the designed size, on a double click and on Home. */
  onReset?: () => void;
  /** What this handle resizes, for anyone not looking at it. */
  label: string;
  className?: string;
  /** Keyboard nudge in pixels, since a divider that only drags is unreachable. */
  step?: number;
}

export function ResizeHandle({
  orientation = "vertical",
  onDrag,
  onEnd,
  onReset,
  label,
  className,
  step = 16,
}: Props) {
  const origin = useRef<number | null>(null);

  return (
    <span
      role="separator"
      aria-orientation={orientation}
      aria-label={label}
      tabIndex={0}
      className={`resize-handle is-${orientation}${className ? ` ${className}` : ""}`}
      onPointerDown={(event) => {
        // Only the primary button, and never the context menu's press: a
        // right click on a divider is somebody reaching for the menu behind it.
        if (event.button !== 0) return;
        event.preventDefault();
        origin.current = orientation === "vertical" ? event.clientX : event.clientY;
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={(event) => {
        if (origin.current === null) return;
        const position = orientation === "vertical" ? event.clientX : event.clientY;
        onDrag(position - origin.current);
      }}
      onPointerUp={(event) => {
        if (origin.current === null) return;
        origin.current = null;
        event.currentTarget.releasePointerCapture(event.pointerId);
        onEnd();
      }}
      onPointerCancel={() => {
        if (origin.current === null) return;
        origin.current = null;
        onEnd();
      }}
      onDoubleClick={onReset}
      onKeyDown={(event) => {
        const nudge =
          event.key === "ArrowLeft" || event.key === "ArrowUp"
            ? -step
            : event.key === "ArrowRight" || event.key === "ArrowDown"
              ? step
              : 0;
        if (nudge !== 0) {
          event.preventDefault();
          onDrag(nudge);
          onEnd();
          return;
        }
        if (event.key === "Home" && onReset) {
          event.preventDefault();
          onReset();
        }
      }}
    />
  );
}
