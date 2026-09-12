import type { CoopFaction, CoopMission, VaultMap } from "../ipc/bindings";
import { useAppStore } from "../store/store";

/** Preview used when a generated map has no rendered map image yet. */
export const GENERATED_MAP_PLACEHOLDER_URL = "/assets/mapgen-placeholder.png";
const LEGACY_GENERATED_MAP_PLACEHOLDER_URL = "/generated-map.svg";

export function isGeneratedMapPlaceholderUrl(url: string | null | undefined): boolean {
  return url === GENERATED_MAP_PLACEHOLDER_URL || url === LEGACY_GENERATED_MAP_PLACEHOLDER_URL;
}

export type MapPresentation = {
  displayName: string;
  thumbnailUrl: string;
  thumbnailUrls: string[];
  coopFaction?: CoopFaction;
  isCoop?: boolean;
};

// Base-game maps are not vault records, so they never appear in the vault
// lookup used by the lobby rows. Keep the same built-in catalog used by the
// reference clients and use the public preview service for their thumbnails.
export type OfficialMapInfo = {
  folderName: string;
  displayName: string;
  maxPlayers: number;
  width: number;
  height: number;
};

export const OFFICIAL_BASE_MAPS: OfficialMapInfo[] = [
  { folderName: "scmp_001", displayName: "Burial Mounds", maxPlayers: 8, width: 1024, height: 1024 },
  { folderName: "scmp_002", displayName: "Concord Lake", maxPlayers: 8, width: 1024, height: 1024 },
  { folderName: "scmp_003", displayName: "Drake's Ravine", maxPlayers: 4, width: 1024, height: 1024 },
  { folderName: "scmp_004", displayName: "Emerald Crater", maxPlayers: 4, width: 1024, height: 1024 },
  { folderName: "scmp_005", displayName: "Gentleman's Reef", maxPlayers: 7, width: 2048, height: 2048 },
  { folderName: "scmp_006", displayName: "Ian's Cross", maxPlayers: 4, width: 1024, height: 1024 },
  { folderName: "scmp_007", displayName: "Open Palms", maxPlayers: 6, width: 512, height: 512 },
  { folderName: "scmp_008", displayName: "Seraphim Glaciers", maxPlayers: 8, width: 1024, height: 1024 },
  { folderName: "scmp_009", displayName: "Seton's Clutch", maxPlayers: 8, width: 1024, height: 1024 },
  { folderName: "scmp_010", displayName: "Sung Island", maxPlayers: 5, width: 1024, height: 1024 },
  { folderName: "scmp_011", displayName: "The Great Void", maxPlayers: 8, width: 2048, height: 2048 },
  { folderName: "scmp_012", displayName: "Theta Passage", maxPlayers: 2, width: 256, height: 256 },
  { folderName: "scmp_013", displayName: "Winter Duel", maxPlayers: 2, width: 256, height: 256 },
  { folderName: "scmp_014", displayName: "The Bermuda Locket", maxPlayers: 8, width: 1024, height: 1024 },
  { folderName: "scmp_015", displayName: "Fields Of Isis", maxPlayers: 4, width: 512, height: 512 },
  { folderName: "scmp_016", displayName: "Canis River", maxPlayers: 2, width: 256, height: 256 },
  { folderName: "scmp_017", displayName: "Syrtis Major", maxPlayers: 4, width: 512, height: 512 },
  { folderName: "scmp_018", displayName: "Sentry Point", maxPlayers: 3, width: 256, height: 256 },
  { folderName: "scmp_019", displayName: "Finn's Revenge", maxPlayers: 2, width: 512, height: 512 },
  { folderName: "scmp_020", displayName: "Roanoke Abyss", maxPlayers: 6, width: 1024, height: 1024 },
  { folderName: "scmp_021", displayName: "Alpha 7 Quarantine", maxPlayers: 8, width: 2048, height: 2048 },
  { folderName: "scmp_022", displayName: "Artic Refuge", maxPlayers: 4, width: 512, height: 512 },
  { folderName: "scmp_023", displayName: "Varga Pass", maxPlayers: 2, width: 512, height: 512 },
  { folderName: "scmp_024", displayName: "Crossfire Canal", maxPlayers: 6, width: 1024, height: 1024 },
  { folderName: "scmp_025", displayName: "Saltrock Colony", maxPlayers: 6, width: 512, height: 512 },
  { folderName: "scmp_026", displayName: "Vya-3 Protectorate", maxPlayers: 4, width: 512, height: 512 },
  { folderName: "scmp_027", displayName: "The Scar", maxPlayers: 6, width: 1024, height: 1024 },
  { folderName: "scmp_028", displayName: "Hanna oasis", maxPlayers: 8, width: 2048, height: 2048 },
  { folderName: "scmp_029", displayName: "Betrayal Ocean", maxPlayers: 8, width: 4096, height: 4096 },
  { folderName: "scmp_030", displayName: "Frostmill Ruins", maxPlayers: 8, width: 4096, height: 4096 },
  { folderName: "scmp_031", displayName: "Four-Leaf Clover", maxPlayers: 4, width: 512, height: 512 },
  { folderName: "scmp_032", displayName: "The Wilderness", maxPlayers: 4, width: 512, height: 512 },
  { folderName: "scmp_033", displayName: "White Fire", maxPlayers: 6, width: 512, height: 512 },
  { folderName: "scmp_034", displayName: "High Noon", maxPlayers: 4, width: 512, height: 512 },
  { folderName: "scmp_035", displayName: "Paradise", maxPlayers: 4, width: 512, height: 512 },
  { folderName: "scmp_036", displayName: "Blasted Rock", maxPlayers: 4, width: 256, height: 256 },
  { folderName: "scmp_037", displayName: "Sludge", maxPlayers: 3, width: 256, height: 256 },
  { folderName: "scmp_038", displayName: "Ambush Pass", maxPlayers: 4, width: 256, height: 256 },
  { folderName: "scmp_039", displayName: "Four-Corners", maxPlayers: 4, width: 256, height: 256 },
  { folderName: "scmp_040", displayName: "The Ditch", maxPlayers: 6, width: 512, height: 512 },
  { folderName: "x1mp_001", displayName: "Crag Dunes", maxPlayers: 8, width: 1024, height: 1024 },
  { folderName: "x1mp_002", displayName: "Williamson's Bridge", maxPlayers: 4, width: 512, height: 512 },
  { folderName: "x1mp_003", displayName: "Snoey Triangle", maxPlayers: 3, width: 512, height: 512 },
  { folderName: "x1mp_004", displayName: "Haven Reef", maxPlayers: 8, width: 1024, height: 1024 },
  { folderName: "x1mp_005", displayName: "The Dark Heart", maxPlayers: 6, width: 1024, height: 1024 },
  { folderName: "x1mp_006", displayName: "Daroza's Sanctuary", maxPlayers: 4, width: 512, height: 512 },
  { folderName: "x1mp_007", displayName: "Strip Mine", maxPlayers: 4, width: 512, height: 512 },
  { folderName: "x1mp_008", displayName: "Thawing Glacier", maxPlayers: 4, width: 1024, height: 1024 },
  { folderName: "x1mp_009", displayName: "Liberiam Battles", maxPlayers: 6, width: 1024, height: 1024 },
  { folderName: "x1mp_010", displayName: "Shards", maxPlayers: 2, width: 256, height: 256 },
  { folderName: "x1mp_011", displayName: "Shuriken Island", maxPlayers: 4, width: 512, height: 512 },
  { folderName: "x1mp_012", displayName: "Debris", maxPlayers: 4, width: 512, height: 512 },
  { folderName: "x1mp_014", displayName: "Flooded Strip Mine", maxPlayers: 6, width: 1024, height: 1024 },
  { folderName: "x1mp_017", displayName: "Eye Of The Storm", maxPlayers: 6, width: 1024, height: 1024 },
];

// Base-game maps are not vault records, so they never appear in the vault
// lookup used by the lobby rows. Keep the same built-in catalog used by the
// reference clients and use the public preview service for their thumbnails.
const OFFICIAL_MAPS: Record<string, string> = Object.fromEntries(
  OFFICIAL_BASE_MAPS.map((m) => [m.folderName, m.displayName]),
);

/** The same catalogue by folder name, for the fields beyond the display name. */
const OFFICIAL_MAPS_BY_FOLDER = new Map(
  OFFICIAL_BASE_MAPS.map((map) => [map.folderName, map] as const),
);

const OFFICIAL_MAP_KEYS_BY_DISPLAY_NAME = new Map(
  Object.entries(OFFICIAL_MAPS).map(([key, displayName]) => [displayName.toLocaleLowerCase(), key]),
);

type VaultMapLookup = {
  byBaseName: Map<string, VaultMap>;
  byDisplayName: Map<string, VaultMap>;
  byFolderName: Map<string, VaultMap>;
};

const VAULT_MAP_LOOKUPS = new WeakMap<VaultMap[], VaultMapLookup>();

function secureUrl(url: string): string {
  return url.trim().replace(/^http:\/\//i, "https://");
}

function uniqueUrls(urls: Array<string | undefined>): string[] {
  return [...new Set(urls.map((url) => secureUrl(url ?? "")).filter(Boolean))];
}

/**
 * Lobby payloads normally contain a map folder, but older servers and test
 * environments may send a scenario path. Reduce all of those forms to the
 * folder name used by the preview service and vault catalogue.
 */
export function normalizeMapName(mapName: string): string {
  const normalized = mapName.trim().replace(/\\/g, "/").replace(/\/+$/, "");
  const parts = normalized.split("/").filter(Boolean);
  const fileName = parts[parts.length - 1] ?? normalized;
  if (/_scenario\.lua$/i.test(fileName) && parts.length > 1) {
    return parts[parts.length - 2].toLocaleLowerCase();
  }
  return fileName
    .replace(/_scenario\.lua$/i, "")
    .replace(/\.(zip|fafmap)$/i, "")
    .toLocaleLowerCase();
}

export function baseMapName(mapName: string): string {
  return normalizeMapName(mapName).replace(/\.v\d+$/i, "");
}

/**
 * The version number a vault map folder carries (`scmp_009.v0004` is 4), or
 * `null` for a folder that states none.
 *
 * Leading zeroes go, because `v0004` is the filesystem's spelling of the
 * number and `v4` is everyone else's. A base-game map, a generated one and a
 * hand-made folder all answer `null`: they have no vault version, and a "v1"
 * invented for them would be a claim nobody made.
 */
export function mapVersionOf(mapName: string): number | null {
  const match = /\.v(\d+)$/i.exec(normalizeMapName(mapName));
  if (!match) return null;
  const version = Number.parseInt(match[1], 10);
  return Number.isFinite(version) && version > 0 ? version : null;
}

function findCoopMission(mapName: string, missions?: CoopMission[]): CoopMission | undefined {
  const coopMissions = missions ?? (typeof window !== "undefined" ? useAppStore.getState?.()?.state?.coop?.missions : []);
  if (!coopMissions || coopMissions.length === 0) return undefined;
  const normalized = normalizeMapName(mapName);
  const baseName = baseMapName(mapName);
  const cleanName = mapName.trim().toLocaleLowerCase();
  return coopMissions.find((m) => {
    const folder = normalizeMapName(m.mapFolderName);
    const missionBase = baseMapName(m.mapFolderName);
    const missionName = m.name.trim().toLocaleLowerCase();
    return (
      folder === normalized ||
      folder === baseName ||
      missionBase === normalized ||
      missionBase === baseName ||
      missionName === cleanName ||
      cleanName.includes(missionName) ||
      missionName.includes(cleanName)
    );
  });
}

function vaultMapLookup(vault: VaultMap[]): VaultMapLookup {
  const cached = VAULT_MAP_LOOKUPS.get(vault);
  if (cached) return cached;

  const lookup: VaultMapLookup = {
    byBaseName: new Map(),
    byDisplayName: new Map(),
    byFolderName: new Map(),
  };
  for (const map of vault) {
    const folderName = normalizeMapName(map.folderName);
    // Preserve Array.find's first-match behavior when aliases collide.
    if (!lookup.byFolderName.has(folderName)) lookup.byFolderName.set(folderName, map);
    const baseName = baseMapName(folderName);
    if (!lookup.byBaseName.has(baseName)) lookup.byBaseName.set(baseName, map);
    const displayName = map.displayName.toLocaleLowerCase();
    if (!lookup.byDisplayName.has(displayName)) lookup.byDisplayName.set(displayName, map);
  }
  VAULT_MAP_LOOKUPS.set(vault, lookup);
  return lookup;
}

/** Twin of `faf_domain::protocol::map_generator::is_generated_map`. */
export function isGeneratedMap(mapName: string): boolean {
  if (!mapName) return false;
  const trimmed = mapName.trim();
  const normalized = normalizeMapName(mapName);
  return (
    /^neroxis_map_generator_.+/i.test(normalized) ||
    /^(neroxis_map_generator_|neroxis\s+map\s+generator\s+).+/i.test(trimmed) ||
    trimmed.toLowerCase().startsWith("neroxis") ||
    trimmed.toLowerCase().includes("neroxis_map_generator")
  );
}

export function extractGeneratedMapSeed(mapName: string): string | undefined {
  if (!mapName) return undefined;
  const clean = mapName.trim().replace(/\.(zip|fafmap)$/i, "");
  const stdMatch = clean.match(/^neroxis_map_generator_[0-9.]+(?:-pre\d+)?_(.+)$/i);
  if (stdMatch && stdMatch[1]) {
    return stdMatch[1];
  }
  const spaceMatch = clean.match(/^neroxis\s+map\s+generator\s+[0-9.]+(?:-pre\d+)?\s+(.+)$/i);
  if (spaceMatch && spaceMatch[1]) {
    return spaceMatch[1];
  }
  const shortMatch = clean.match(/^neroxis[_\s]+([0-9.]+[_\s]+)?(.+)$/i);
  if (shortMatch && shortMatch[2]) {
    const candidate = shortMatch[2].trim();
    if (candidate.toLowerCase() !== "map generator" && candidate.toLowerCase() !== "map_generator") {
      return candidate;
    }
  }
  return undefined;
}

/**
 * What the backend writes into a vault listing when the API named no map at
 * all. Kept as a value rather than an empty string so a listing that never had
 * a map stays distinguishable from a field that was dropped on the way here.
 */
export const UNKNOWN_VAULT_MAP = "unknown map";

/** Did this listing arrive without a map? */
export function isUnknownVaultMap(mapName: string): boolean {
  const trimmed = mapName.trim();
  return trimmed === "" || trimmed.toLocaleLowerCase() === UNKNOWN_VAULT_MAP;
}

/**
 * Prefer the technical map name from a matching local replay when a vault
 * replay only contains the generic Neroxis display name. The technical name
 * is the key used by the locally generated preview cache.
 *
 * The same file answers a second question the vault cannot: a co-op game has
 * no map version in the API, so the listing carries no map, while the replay
 * header names the mission's own folder.
 */
export function effectiveReplayMapName(replayMap: string, localMap?: string | null): string {
  if (localMap && isGeneratedMap(localMap) && Boolean(extractGeneratedMapSeed(localMap))) {
    return localMap;
  }
  if (localMap && localMap.trim() && isUnknownVaultMap(replayMap)) return localMap;
  return replayMap;
}

function fallbackDisplayName(mapName: string): string {
  return baseMapName(mapName)
    .replace(/_/g, " ")
    .replace(/\b\w/g, (letter: string) => letter.toLocaleUpperCase());
}

export function findVaultMap(vault: VaultMap[], mapName: string): VaultMap | undefined {
  return findVaultMapByFolder(vault, mapName)
    ?? vaultMapLookup(vault).byDisplayName.get(mapName.trim().toLocaleLowerCase());
}

/**
 * The catalogue entry whose *folder* a map name refers to, ignoring the
 * `.vNNNN` the name may carry and the catalogue's folder does not.
 *
 * Separate from [`findVaultMap`] because a caller deciding whether a game is
 * ranked must not fall through to a display-name match: the name it holds came
 * off the wire as a folder, and a coincidental title collision would quietly
 * mark somebody's game unranked.
 */
export function findVaultMapByFolder(vault: VaultMap[], mapName: string): VaultMap | undefined {
  const lookup = vaultMapLookup(vault);
  return lookup.byFolderName.get(normalizeMapName(mapName))
    ?? lookup.byBaseName.get(baseMapName(mapName));
}

export function mapThumbnailCandidates(
  vault: VaultMap[],
  mapName: string,
  large = false,
  missions?: CoopMission[],
  customGeneratedPreview?: string,
  customUrl?: string,
  preferCanonicalPreview = false,
): string[] {
  const isGen = isGeneratedMap(mapName);
  const normalized = normalizeMapName(mapName);

  if (isGen) {
    const generatedPreview =
      customGeneratedPreview ??
      (typeof window !== "undefined"
        ? useAppStore.getState?.()?.state?.mapGenerator?.previews?.[mapName] ||
          useAppStore.getState?.()?.state?.mapGenerator?.previews?.[normalized] ||
          useAppStore.getState?.()?.state?.mapGenerator?.previews?.[mapName.toLowerCase()]
        : undefined);

    return uniqueUrls([
      generatedPreview,
      customUrl && !isGeneratedMapPlaceholderUrl(customUrl) ? customUrl : undefined,
      GENERATED_MAP_PLACEHOLDER_URL,
    ]);
  }

  const vaultMap = findVaultMap(vault, mapName);
  const coopMission = findCoopMission(mapName, missions);
  const baseName = baseMapName(mapName);
  const officialKey = OFFICIAL_MAPS[baseName]
    ? baseName
    : OFFICIAL_MAP_KEYS_BY_DISPLAY_NAME.get(mapName.trim().toLocaleLowerCase());
  const size = large ? "large" : "small";
  const canonicalPreviewUrls = [
    officialKey
      ? `https://content.faforever.com/maps/previews/${size}/${officialKey}.png`
      : undefined,
    normalized && !normalized.includes(" ")
      ? `https://content.faforever.com/maps/previews/${size}/${encodeURIComponent(normalized)}.png`
      : undefined,
    baseName !== normalized && !baseName.includes(" ")
      ? `https://content.faforever.com/maps/previews/${size}/${encodeURIComponent(baseName)}.png`
      : undefined,
  ];
  // Chat uses the same unmarked small art as the reference clients. Do not
  // fall through to a large preview there: large map art can include resource
  // and structure markers that are useful in map details but noisy at 22px.
  const coopPreviewUrls = large
    ? [coopMission?.thumbnailUrlLarge, coopMission?.thumbnailUrlSmall]
    : preferCanonicalPreview
      ? [coopMission?.thumbnailUrlSmall]
      : [coopMission?.thumbnailUrlSmall, coopMission?.thumbnailUrlLarge];
  const vaultPreviewUrls = large
    ? [vaultMap?.thumbnailUrlLarge, vaultMap?.thumbnailUrl]
    : preferCanonicalPreview
      ? [vaultMap?.thumbnailUrl]
      : [vaultMap?.thumbnailUrl, vaultMap?.thumbnailUrlLarge];

  return uniqueUrls([
    customUrl,
    ...(preferCanonicalPreview ? canonicalPreviewUrls : []),
    ...coopPreviewUrls,
    ...vaultPreviewUrls,
    ...(!preferCanonicalPreview ? canonicalPreviewUrls : []),
  ]);
}

export function inferCoopFaction(mapName: string): CoopFaction {
  const normalized = normalizeMapName(mapName);
  if (
    normalized.includes("scca_coop_e") ||
    normalized.includes("uef") ||
    normalized.includes("theta") ||
    normalized.includes("earth") ||
    normalized.includes("black_day") ||
    normalized.includes("black day") ||
    normalized.includes("procyon")
  ) return "uef";
  if (
    normalized.includes("scca_coop_c") ||
    normalized.includes("cybran") ||
    normalized.includes("symbiont") ||
    normalized.includes("brackman") ||
    normalized.includes("hex5") ||
    normalized.includes("qai")
  ) return "cybran";
  if (
    normalized.includes("scca_coop_a") ||
    normalized.includes("aeon") ||
    normalized.includes("holy") ||
    normalized.includes("princess") ||
    normalized.includes("crusade") ||
    normalized.includes("illuminate")
  ) return "aeon";
  if (
    normalized.includes("x1ca_coop_") ||
    normalized.includes("seraphim") ||
    normalized.includes("alien") ||
    normalized.includes("ou-eatha")
  ) return "seraphim";
  return "custom";
}

export function mapPresentation(vault: VaultMap[], mapName: string, missions?: CoopMission[]): MapPresentation {
  const vaultMap = findVaultMap(vault, mapName);
  const coopMission = findCoopMission(mapName, missions);
  const baseName = baseMapName(mapName);
  const officialEntry = OFFICIAL_MAPS[baseName]
    ? ([baseName, OFFICIAL_MAPS[baseName]] as const)
    : (() => {
        const key = OFFICIAL_MAP_KEYS_BY_DISPLAY_NAME.get(mapName.trim().toLocaleLowerCase());
        return key ? ([key, OFFICIAL_MAPS[key]] as const) : undefined;
      })();
  const thumbnailUrls = mapThumbnailCandidates(vault, mapName, false, missions);

  if (isGeneratedMap(mapName)) {
    return {
      displayName: "Neroxis Map Generator",
      thumbnailUrl: thumbnailUrls[0] || GENERATED_MAP_PLACEHOLDER_URL,
      thumbnailUrls: thumbnailUrls.length > 0 ? thumbnailUrls : [GENERATED_MAP_PLACEHOLDER_URL],
    };
  }

  if (coopMission) {
    const coopFaction = coopMission.scenarioId
      ? (typeof window !== "undefined"
          ? useAppStore.getState?.()?.state?.coop?.scenarios?.find((s) => s.id === coopMission.scenarioId)?.faction
          : undefined) ?? inferCoopFaction(coopMission.mapFolderName)
      : inferCoopFaction(coopMission.mapFolderName);
    return {
      displayName: coopMission.name,
      thumbnailUrl: thumbnailUrls[0] ?? "",
      thumbnailUrls,
      isCoop: true,
      coopFaction,
    };
  }

  const isCoopMap = /^(scca_coop_|x1ca_coop_)/i.test(normalizeMapName(mapName)) || mapName.toLowerCase().includes("coop");
  if (isCoopMap) {
    return {
      displayName: fallbackDisplayName(mapName),
      thumbnailUrl: thumbnailUrls[0] ?? "",
      thumbnailUrls,
      isCoop: true,
      coopFaction: inferCoopFaction(mapName),
    };
  }

  if (officialEntry) {
    return {
      displayName: officialEntry[1],
      thumbnailUrl: thumbnailUrls[0] ?? "",
      thumbnailUrls,
    };
  }
  if (vaultMap) {
    return {
      displayName: vaultMap.displayName,
      thumbnailUrl: thumbnailUrls[0] ?? "",
      thumbnailUrls,
    };
  }

  return {
    displayName: fallbackDisplayName(mapName),
    thumbnailUrl: thumbnailUrls[0] ?? "",
    thumbnailUrls,
  };
}

// ── Map size ────────────────────────────────────────────────────────────────
//
// Asked for on the game tiles: the list said which map a game was on and not
// how big it is, which is most of what "do I want this game" turns on. The
// size is not on the lobby's `Game` record at all, so it comes from whichever
// of the three sources knows this particular map.

/** Generator units per kilometre. A 512-unit map is the familiar 10 km one. */
const UNITS_PER_KM = 51.2;

export type MapSize = {
  /** `10 × 10 km`, for a tooltip or a facts list. */
  full: string;
  /**
   * `10 km` for the square maps that almost all of them are, `10 × 5 km`
   * otherwise. For the badge on a tile, where the space is a corner of a
   * thumbnail and "10 × 10 km" spends most of it repeating itself.
   */
  compact: string;
};

function formatKm(units: number): string {
  return (units / UNITS_PER_KM).toFixed(0);
}

/** Both renderings of a width and height in generator units. */
export function mapSizeOf(width: number, height: number): MapSize | null {
  if (!(width > 0) || !(height > 0)) return null;
  const w = formatKm(width);
  const h = formatKm(height);
  return {
    full: `${w} × ${h} km`,
    compact: w === h ? `${w} km` : `${w} × ${h} km`,
  };
}

/**
 * How big the map a game is on is, from whichever source knows it.
 *
 * The vault first, because it is the authority for anything uploaded. Then the
 * built-in table for the base-game maps, which are not vault records and so
 * never appear in a vault lookup. Then, for a generated map, the name itself:
 * a Neroxis name carries its size, and `decodedSize` is that number once the
 * map generator slice has decoded the name.
 *
 * `null` where none of the three knows, which is honest: a badge saying
 * nothing is better than one guessing 10 km because that is the common case.
 */
export function mapSize(
  vault: VaultMap[],
  mapName: string,
  decodedSize?: number,
): MapSize | null {
  if (isGeneratedMap(mapName)) {
    return decodedSize ? mapSizeOf(decodedSize, decodedSize) : null;
  }
  const vaultMap = findVaultMap(vault, mapName);
  if (vaultMap) {
    const size = mapSizeOf(vaultMap.width, vaultMap.height);
    if (size) return size;
  }
  const official = OFFICIAL_MAPS_BY_FOLDER.get(baseMapName(mapName));
  return official ? mapSizeOf(official.width, official.height) : null;
}
