// Draggable column widths, stored per table.
//
// The replay list grew this first and the live-replay and co-op tables wanted
// the same thing, which is the moment to have one of it rather than three.
// Every table brings its own designed widths and its own settings field; the
// clamping, the "local while dragging, saved on release" split, and the reset
// are the same for all of them.

import { useState } from "react";
import type { BrowsingPreferences } from "../ipc/bindings";
import { ipc } from "../ipc/client";
import { useAppStore } from "../store/store";
import { MAX_BROWSER_COLUMN_PX, MIN_BROWSER_COLUMN_PX } from "./browsingPreferences";

/** The settings fields that hold a table's column widths. */
type ColumnField = {
  [K in keyof BrowsingPreferences]: BrowsingPreferences[K] extends number[] ? K : never;
}[keyof BrowsingPreferences];

export interface ColumnWidths {
  /** What to draw with now: the drag in progress, or what was saved. */
  widths: number[];
  /** Move one column by this many pixels. Bounded, so a drag cannot erase it. */
  onDrag: (index: number, delta: number) => void;
  /** The drag ended: persist it. */
  onCommit: () => void;
  /** Back to the designed widths, and stay there across a restart. */
  onReset: () => void;
}

/**
 * @param field which browsing preference holds this table's widths
 * @param defaults the designed widths, in the order the columns are drawn
 */
export function useColumnWidths(field: ColumnField, defaults: readonly number[]): ColumnWidths {
  const stored = useAppStore((state) => state.state.settings.browsing[field]);
  // Local until the pointer is released: persisting per frame would write a
  // settings file on every mouse move.
  const [dragged, setDragged] = useState<number[] | null>(null);

  const resolve = () =>
    defaults.map((fallback, index) => {
      const saved = stored?.[index];
      return saved && saved > 0
        ? Math.min(MAX_BROWSER_COLUMN_PX, Math.max(MIN_BROWSER_COLUMN_PX, Math.round(saved)))
        : fallback;
    });

  const save = (widths: number[]) => {
    const current = useAppStore.getState().state.settings.browsing;
    ipc.send({
      kind: "Settings",
      command: { type: "setBrowsing", payload: { preferences: { ...current, [field]: widths } } },
    });
  };

  return {
    widths: dragged ?? resolve(),
    onDrag: (index, delta) =>
      setDragged((current) =>
        (current ?? resolve()).map((width, position) =>
          position === index
            ? Math.min(
                MAX_BROWSER_COLUMN_PX,
                Math.max(MIN_BROWSER_COLUMN_PX, Math.round(width + delta)),
              )
            : width,
        ),
      ),
    onCommit: () => {
      if (dragged) save(dragged);
      setDragged(null);
    },
    // An empty array is the reset: the backend keeps it and the resolve above
    // reads it back as "use the designed widths", so a reset survives a
    // restart the same way a drag does.
    onReset: () => {
      setDragged(null);
      save([]);
    },
  };
}
