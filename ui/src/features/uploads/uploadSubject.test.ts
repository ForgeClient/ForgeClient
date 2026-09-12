import { describe, expect, it } from "vitest";
import type { InstalledMap, InstalledMod, UploadRequest } from "../../ipc/bindings";
import { uploadDescription, uploadFacts, vaultPresence } from "./uploadSubject";

const request = (over: Partial<UploadRequest> = {}): UploadRequest => ({
  kind: "mod",
  folderName: "nomads",
  displayName: "Nomads",
  ranked: false,
  sourcePath: null,
  renameTo: "",
  ...over,
});

const size = (bytes: number) => `${bytes} B`;

describe("whether the vault already has this", () => {
  it("calls a second upload of the same name an update", () => {
    // The report: updating a mod announced itself as uploading a new one.
    expect(vaultPresence(request(), [{ displayName: "Nomads" }], true)).toBe("update");
    expect(vaultPresence(request(), [{ displayName: " nomads " }], true)).toBe("update");
  });

  it("calls a name the vault has never seen new", () => {
    expect(vaultPresence(request(), [{ displayName: "Total Mayhem" }], true)).toBe("new");
  });

  it("says nothing at all until the catalogue has loaded", () => {
    // Claiming "new upload" from an empty cache is the exact mistake being
    // fixed, so an unloaded catalogue is its own answer.
    expect(vaultPresence(request(), [], false)).toBe("unknown");
  });

  it("treats a rename as new, because FAF has no rename", () => {
    expect(
      vaultPresence(request({ renameTo: "Nomads Redux" }), [{ displayName: "Nomads" }], true),
    ).toBe("new");
  });
});

describe("the facts an author can check before publishing", () => {
  const mod: InstalledMod = {
    folderName: "nomads",
    uid: "abc-123",
    displayName: "Nomads",
    version: "42",
    author: "Exotic_Retard",
    description: "A fourth faction.",
    modType: "sim",
    enabled: true,
  };

  it("lists a mod's version, author and uid", () => {
    expect(uploadFacts(request(), mod, null, size)).toEqual([
      { labelKey: "uploads.fact.name", value: "Nomads" },
      { labelKey: "uploads.fact.version", value: "42" },
      { labelKey: "uploads.fact.author", value: "Exotic_Retard" },
      { labelKey: "uploads.fact.uid", value: "abc-123" },
      { labelKey: "uploads.fact.folder", value: "nomads" },
    ]);
  });

  it("gives a map its size in kilometres and its player count", () => {
    const map: InstalledMap = {
      folderName: "scmp_009.v0001",
      displayName: "Seton's Clutch",
      maxPlayers: 8,
      width: 1024,
      height: 1024,
      version: "1",
      description: null,
    };
    const facts = uploadFacts(
      request({ kind: "map", folderName: "scmp_009.v0001", displayName: "Seton's Clutch" }),
      map,
      null,
      size,
    );
    expect(facts).toContainEqual({ labelKey: "uploads.fact.mapSize", value: "20 km" });
    expect(facts).toContainEqual({ labelKey: "uploads.fact.players", value: "8" });
  });

  it("leaves out what it does not know rather than printing a dash", () => {
    // A folder picked from disk has no installed record behind it.
    expect(uploadFacts(request({ sourcePath: "D:/mods/wip" }), undefined, null, size)).toEqual([
      { labelKey: "uploads.fact.name", value: "Nomads" },
      { labelKey: "uploads.fact.folder", value: "D:/mods/wip" },
    ]);
  });

  it("shows the size once compression has measured it", () => {
    const facts = uploadFacts(request(), mod, 2048, size);
    expect(facts).toContainEqual({ labelKey: "uploads.fact.size", value: "2048 B" });
  });

  it("returns no description for a blank one", () => {
    expect(uploadDescription(mod)).toBe("A fourth faction.");
    expect(uploadDescription({ ...mod, description: "   " })).toBeNull();
    expect(uploadDescription(undefined)).toBeNull();
  });
});
