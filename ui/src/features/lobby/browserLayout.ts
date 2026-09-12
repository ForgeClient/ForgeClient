// How wide the game list's columns and the detail panel beside them are.
//
// The tiles could already be made narrower or wider (`gameTileColumns`); the
// list's columns and the panel next to them could not, so the only way to read
// a long lobby title was to hope. Both are arithmetic rather than rendering,
// so both live here with the bounds the backend enforces anyway.

import {
  MAX_BROWSER_COLUMN_PX,
  MAX_BROWSER_COLUMNS,
  MAX_DETAIL_PX,
  MIN_BROWSER_COLUMN_PX,
  MIN_DETAIL_PX,
} from "../../shared/browsingPreferences";

/**
 * The designed widths, in the order the header lists them: game, map, players,
 * rating, age. The first is the flexible one, so its number is a starting
 * point rather than a ceiling.
 */
export const DEFAULT_COLUMN_WIDTHS: readonly number[] = [320, 170, 80, 100, 75];

/** The detail panel's designed width, matching `custom-games.css`. */
export const DEFAULT_DETAIL_WIDTH = 270;

/**
 * Saved widths, padded and bounded into a usable set of five.
 *
 * A stored array can be short (a release that had fewer columns), long (one
 * that had more), or empty (nobody has ever dragged anything). All three mean
 * "use the designed width for the columns you do not know about", which is why
 * this pads rather than rejects.
 */
export function columnWidths(stored: readonly number[] | undefined): number[] {
  return DEFAULT_COLUMN_WIDTHS.map((fallback, index) => {
    const saved = stored?.[index];
    return saved && saved > 0
      ? Math.min(MAX_BROWSER_COLUMN_PX, Math.max(MIN_BROWSER_COLUMN_PX, Math.round(saved)))
      : fallback;
  }).slice(0, MAX_BROWSER_COLUMNS);
}

/**
 * One column resized by `delta` pixels, the others untouched.
 *
 * Deliberately not a pair-wise resize that steals from the neighbour: the
 * first column is the flexible one, so taking width from the column to the
 * right of the one being dragged would make the *title* jump every time
 * somebody widened the map column, which reads as a bug rather than a feature.
 */
export function withColumnResized(
  widths: readonly number[],
  index: number,
  delta: number,
): number[] {
  return widths.map((width, position) =>
    position === index
      ? Math.min(MAX_BROWSER_COLUMN_PX, Math.max(MIN_BROWSER_COLUMN_PX, Math.round(width + delta)))
      : width,
  );
}

/**
 * The CSS `grid-template-columns` for a set of widths.
 *
 * The first track stays flexible so the list always fills its panel: its saved
 * width becomes the minimum it may shrink to, not the width it must be.
 */
export function columnTemplate(widths: readonly number[]): string {
  return widths
    .map((width, index) => (index === 0 ? `minmax(${width}px, 2.4fr)` : `${width}px`))
    .join(" ");
}

/** The detail panel's width after a drag, bounded the way the backend bounds it. */
export function withDetailResized(width: number, delta: number): number {
  // The handle sits on the panel's left edge, so dragging left (a negative
  // delta) makes the panel wider.
  return Math.min(MAX_DETAIL_PX, Math.max(MIN_DETAIL_PX, Math.round(width - delta)));
}

/** The stored detail width, or the designed one when nothing is stored. */
export function detailWidth(stored: number | undefined): number {
  return stored && stored > 0
    ? Math.min(MAX_DETAIL_PX, Math.max(MIN_DETAIL_PX, stored))
    : DEFAULT_DETAIL_WIDTH;
}
