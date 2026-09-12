import { describe, expect, it } from "vitest";
import {
  columnTemplate,
  columnWidths,
  DEFAULT_COLUMN_WIDTHS,
  DEFAULT_DETAIL_WIDTH,
  detailWidth,
  withColumnResized,
  withDetailResized,
} from "./browserLayout";
import {
  MAX_BROWSER_COLUMN_PX,
  MAX_DETAIL_PX,
  MIN_BROWSER_COLUMN_PX,
  MIN_DETAIL_PX,
} from "../../shared/browsingPreferences";

describe("the game list's column widths", () => {
  it("falls back to the designed width for anything not stored", () => {
    // Empty, short and over-long are all "use the design for the rest": a
    // release that adds or drops a column must not leave the list unusable.
    expect(columnWidths([])).toEqual([...DEFAULT_COLUMN_WIDTHS]);
    expect(columnWidths([400])).toEqual([400, ...DEFAULT_COLUMN_WIDTHS.slice(1)]);
    expect(columnWidths([400, 0, 0, 0, 0, 999])).toEqual([
      400,
      ...DEFAULT_COLUMN_WIDTHS.slice(1),
    ]);
    expect(columnWidths(undefined)).toEqual([...DEFAULT_COLUMN_WIDTHS]);
  });

  it("resizes only the column being dragged", () => {
    // Stealing from the neighbour would make the title jump every time
    // somebody widened the map column.
    const widths = [320, 170, 80, 100, 75];
    expect(withColumnResized(widths, 1, 40)).toEqual([320, 210, 80, 100, 75]);
    expect(withColumnResized(widths, 1, -40)).toEqual([320, 130, 80, 100, 75]);
  });

  it("keeps every column within reach of another drag", () => {
    const widths = [320, 170, 80, 100, 75];
    expect(withColumnResized(widths, 2, -5000)[2]).toBe(MIN_BROWSER_COLUMN_PX);
    expect(withColumnResized(widths, 2, 5000)[2]).toBe(MAX_BROWSER_COLUMN_PX);
  });

  it("leaves the first column flexible so the list still fills the panel", () => {
    expect(columnTemplate([320, 170, 80, 100, 75])).toBe(
      "minmax(320px, 2.4fr) 170px 80px 100px 75px",
    );
  });
});

describe("the details panel's width", () => {
  it("widens as the divider is dragged towards the list", () => {
    // The handle is on the panel's left edge, so left is wider.
    expect(withDetailResized(300, -40)).toBe(340);
    expect(withDetailResized(300, 40)).toBe(260);
  });

  it("stays between a readable preview and a usable game list", () => {
    expect(withDetailResized(300, -5000)).toBe(MAX_DETAIL_PX);
    expect(withDetailResized(300, 5000)).toBe(MIN_DETAIL_PX);
  });

  it("reads an unset width as the designed one", () => {
    expect(detailWidth(0)).toBe(DEFAULT_DETAIL_WIDTH);
    expect(detailWidth(undefined)).toBe(DEFAULT_DETAIL_WIDTH);
    expect(detailWidth(400)).toBe(400);
  });
});
