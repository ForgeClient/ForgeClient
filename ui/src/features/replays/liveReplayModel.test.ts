import { describe, expect, it } from "vitest";
import type { Game } from "../../ipc/bindings";
import {
  DEFAULT_LIVE_FILTERS,
  liveFeaturedModOptions,
  parseLiveFilters,
  prettyFeaturedMod,
} from "./liveReplayModel";

describe("live replay filter persistence", () => {
  it("accepts only expected fields with the expected primitive types", () => {
    expect(parseLiveFilters({
      search: "ranked",
      hideModded: true,
      maxPlayers: 12,
      friendsOnly: "yes",
      injected: "ignored",
    })).toEqual({
      ...DEFAULT_LIVE_FILTERS,
      search: "ranked",
      hideModded: true,
    });
  });

  it.each([null, [], "filters", 42])("falls back for non-record value %p", (value) => {
    expect(parseLiveFilters(value)).toEqual(DEFAULT_LIVE_FILTERS);
  });
});

describe("the featured mod filter", () => {
  const game = (modName: string) => ({ modName }) as Game;

  it("leaves co-op to the game type filter", () => {
    // Both dropdowns offered the same games, spelled "Co-op" in one and
    // "coop" in the other, which read as two different sections.
    expect(liveFeaturedModOptions([game("faf"), game("coop"), game("Coop")]))
      .toEqual(["faf"]);
  });

  it("offers every other mod the server is actually running", () => {
    expect(liveFeaturedModOptions([game("nomads"), game("faf"), game("faf"), game("")]))
      .toEqual(["faf", "nomads"]);
  });

  it("spells a known mod the way the host dialog does", () => {
    expect(prettyFeaturedMod("faf")).toBe("FAF");
    expect(prettyFeaturedMod("nomads")).toBe("Nomads");
    // An unknown technical name is capitalised, never invented.
    expect(prettyFeaturedMod("murderparty")).toBe("Murderparty");
  });
});
