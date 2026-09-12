// The mods column, identical in both host dialogs: the UI/Sim split, a search,
// saved presets, and bulk actions scoped to whichever kind is on screen.
//
// Laid out to match the map column beside it, which was the request: the kind
// switcher sits where the All/Favourites tabs sit, the search under it, and
// the presets button where the map filter is. The two columns had the same
// parts in a different order, which reads as two designs rather than one.
//
// Self-contained on purpose: it reads the installed mods and the presets from
// the store and writes both back itself, so a dialog embedding it passes
// nothing and cannot get the wiring subtly wrong.

import { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../../../design-system/Button";
import { Icon } from "../../../design-system/Icon";
import { ipc } from "../../../ipc/client";
import type { InstalledMod, ModPreset } from "../../../ipc/bindings";
import { useTranslation } from "../../../i18n/useTranslation";
import { useAppStore } from "../../../store/store";
import { ModPresetModal } from "../ModPresetModal";

type ModTab = "ui" | "sim";

export function HostModsColumn() {
  const { t } = useTranslation();
  const installedMods = useAppStore((state) => state.state.mods.installed);
  const presets = useAppStore((state) => state.state.settings.browsing.modPresets);

  const [modTab, setModTab] = useState<ModTab>("ui");
  const [modSearch, setModSearch] = useState("");
  const [presetModalOpen, setPresetModalOpen] = useState(false);
  const [presetsOpen, setPresetsOpen] = useState(false);
  const presetRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!presetsOpen) return;
    const closeOnOutsideClick = (event: MouseEvent) => {
      if (event.target instanceof Node && !presetRef.current?.contains(event.target)) {
        setPresetsOpen(false);
      }
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setPresetsOpen(false);
    };
    document.addEventListener("mousedown", closeOnOutsideClick);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOnOutsideClick);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [presetsOpen]);

  /// `setBrowsing` replaces the whole preferences bag, so a writer must start
  /// from the newest copy rather than the one captured at render time. Saving a
  /// preset and closing the dialog in quick succession would otherwise write the
  /// preset straight back out again.
  const currentBrowsing = () => useAppStore.getState().state.settings.browsing;

  const activeModsCount = useMemo(
    () => installedMods.filter((mod) => mod.enabled).length,
    [installedMods],
  );
  const simMods = useMemo(
    () => installedMods.filter((mod) => mod.modType === "sim"),
    [installedMods],
  );
  const uiMods = useMemo(() => installedMods.filter((mod) => mod.modType === "ui"), [installedMods]);

  const filteredMods = useMemo(() => {
    const query = modSearch.trim().toLowerCase();
    return installedMods
      .filter((mod) => {
        if (mod.modType !== modTab) return false;
        if (!query) return true;
        return (
          mod.displayName.toLowerCase().includes(query) ||
          (mod.modType === "ui" && query === "ui") ||
          (mod.modType === "sim" && query === "sim")
        );
      })
      .sort(
        (a, b) =>
          Number(b.enabled) - Number(a.enabled) || a.displayName.localeCompare(b.displayName),
      );
  }, [installedMods, modSearch, modTab]);

  // One command rather than one per mod: each `toggleMod` rewrites
  // `game.prefs` and rescans every mod folder, so a bulk action across twenty
  // mods would cost twenty rescans and walk the list through every
  // intermediate state on screen.
  const setActiveMods = (uids: string[]) => {
    ipc.send({ kind: "Mods", command: { type: "setActiveMods", payload: { uids } } });
  };

  const setModsEnabled = (mods: InstalledMod[], enabled: boolean) => {
    const affected = new Set(mods.map((mod) => mod.uid));
    setActiveMods(
      installedMods
        .filter((mod) => (affected.has(mod.uid) ? enabled : mod.enabled))
        .map((mod) => mod.uid),
    );
  };

  // A preset is the complete wanted state, so everything it does not name is
  // switched off. Without that, "put my set back" would leave whatever the user
  // had enabled in the meantime sitting alongside it.
  const applyPreset = (preset: ModPreset) => {
    const wanted = new Set(preset.uids);
    setActiveMods(installedMods.filter((mod) => wanted.has(mod.uid)).map((mod) => mod.uid));
  };

  const persistPresets = (modPresets: ModPreset[]) => {
    ipc.send({
      kind: "Settings",
      command: {
        type: "setBrowsing",
        payload: { preferences: { ...currentBrowsing(), modPresets } },
      },
    });
  };

  const savePreset = (name: string, uids: string[]) => {
    const key = name.toLocaleLowerCase();
    const existing = presets.findIndex((preset) => preset.name.toLocaleLowerCase() === key);
    // Overwrite in place rather than moving the preset to the end: the list is
    // a row of buttons, and having one jump position on every save is worse
    // than it sounds once there are more than two.
    persistPresets(
      existing >= 0
        ? presets.map((preset, index) => (index === existing ? { name, uids } : preset))
        : [...presets, { name, uids }],
    );
    setPresetModalOpen(false);
  };

  const deletePreset = (name: string) =>
    persistPresets(presets.filter((preset) => preset.name !== name));

  // What the list shows is what Enable All / Disable All act on, which is how
  // the two mod kinds stay separately controllable without four buttons.
  const tabMods = modTab === "ui" ? uiMods : simMods;

  return (
    <section className="host-column host-column-mods surface-panel">
      <div className="host-column-header">
        <h3>{t("lobby.host.mods")}</h3>
        <div className="host-header-actions">
          <span className="host-count-badge">
            {t("lobby.host.modCount", {
              active: activeModsCount,
              installed: installedMods.length,
            })}
          </span>
          <button
            type="button"
            className="host-header-icon-btn"
            title={t("lobby.host.reloadMods")}
            aria-label={t("lobby.host.reloadMods")}
            onClick={() => ipc.send({ kind: "Mods", command: { type: "loadInstalled" } })}
          >
            <Icon name="refresh" size={12} />
          </button>
        </div>
      </div>

      {/* The same switch the map column uses, rather than a second one that
          looks nearly like it. Two controls doing the same job in one dialog
          should not be distinguishable by their styling. */}
      <div className="host-mod-tabs section-tabs" role="tablist" aria-label={t("lobby.host.mods")}>
        <button
          type="button"
          role="tab"
          aria-selected={modTab === "ui"}
          className={modTab === "ui" ? "active" : ""}
          onClick={() => setModTab("ui")}
        >
          {t("lobby.host.uiMods")} ({uiMods.length})
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={modTab === "sim"}
          className={modTab === "sim" ? "active" : ""}
          onClick={() => setModTab("sim")}
        >
          {t("lobby.host.simMods")} ({simMods.length})
        </button>
      </div>
      <div className="host-map-search-row">
        <div className="search-field host-column-search host-map-search-field">
          <Icon name="search" size={13} />
          <input
            value={modSearch}
            onChange={(event) => setModSearch(event.target.value)}
            placeholder={t("lobby.host.searchModsPlaceholder")}
            aria-label={t("lobby.host.searchModsAria")}
          />
        </div>
        <div className="host-map-filter" ref={presetRef}>
          <button
            type="button"
            className={`host-map-filter-button${presets.length > 0 ? " active" : ""}`}
            aria-expanded={presetsOpen}
            onClick={() => setPresetsOpen((open) => !open)}
          >
            <Icon name="mods" size={13} />
            {t("lobby.host.presets")}
            {presets.length > 0 && (
              <span className="host-map-filter-count">{presets.length}</span>
            )}
          </button>

          {presetsOpen && (
            <div className="host-preset-popover surface-panel" role="group">
              <div className="host-preset-popover-header">
                <span>{t("lobby.host.presets")}</span>
                <button
                  type="button"
                  className="host-preset-save-btn"
                  disabled={installedMods.length === 0}
                  onClick={() => {
                    setPresetsOpen(false);
                    setPresetModalOpen(true);
                  }}
                >
                  <Icon name="plus" size={12} /> {t("lobby.host.savePreset")}
                </button>
              </div>
              {presets.length === 0 ? (
                <p className="host-preset-popover-empty">{t("lobby.host.noPresets")}</p>
              ) : (
                <div className="host-preset-popover-list">
                  {presets.map((preset) => (
                    <div key={preset.name} className="host-preset-popover-item">
                      <button
                        type="button"
                        className="host-preset-popover-apply"
                        title={t("lobby.host.applyPreset", { name: preset.name })}
                        onClick={() => {
                          applyPreset(preset);
                          setPresetsOpen(false);
                        }}
                      >
                        <span className="host-preset-popover-name">{preset.name}</span>
                        <span className="host-preset-popover-count">
                          {t("lobby.host.presetModCount", { count: preset.uids.length })}
                        </span>
                      </button>
                      <button
                        type="button"
                        className="host-preset-popover-delete"
                        aria-label={t("lobby.host.deletePreset", { name: preset.name })}
                        onClick={() => deletePreset(preset.name)}
                      >
                        <Icon name="close" size={11} />
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      </div>


      <div className="host-column-body host-mod-list">
        {filteredMods.length === 0 ? (
          <p className="play-empty">
            {t(installedMods.length === 0 ? "lobby.host.noInstalledMods" : "lobby.host.noModsMatch")}
          </p>
        ) : (
          filteredMods.map((mod) => (
            <label key={mod.uid} className={`host-mod-row${mod.enabled ? " is-active" : ""}`}>
              <input
                type="checkbox"
                checked={mod.enabled}
                onChange={(event) =>
                  ipc.send({
                    kind: "Mods",
                    command: {
                      type: "toggleMod",
                      payload: { uid: mod.uid, enabled: event.target.checked },
                    },
                  })
                }
              />
              <span className="host-mod-name" title={mod.displayName}>
                {mod.displayName}
              </span>
              {/* The tab already says which kind these are, so the trailing
                  slot carries the version instead of a redundant badge. The
                  `v` is not decoration: a bare "21" beside a mod name reads as
                  a count of something, and nobody could tell what. */}
              <span
                className="host-mod-version"
                title={t("lobby.host.modVersion", { version: mod.version })}
              >
                v{mod.version}
              </span>
            </label>
          ))
        )}
      </div>

      <div className="host-column-footer host-mod-actions">
        <Button
          className="host-col-action-btn"
          disabled={tabMods.length === 0 || tabMods.every((mod) => mod.enabled)}
          onClick={() => setModsEnabled(tabMods, true)}
        >
          {t("lobby.host.enableAll")}
        </Button>
        <Button
          className="host-col-action-btn"
          disabled={!tabMods.some((mod) => mod.enabled)}
          onClick={() => setModsEnabled(tabMods, false)}
        >
          {t("lobby.host.disableAll")}
        </Button>
      </div>

      {presetModalOpen && (
        <ModPresetModal
          installedMods={installedMods}
          presets={presets}
          onCancel={() => setPresetModalOpen(false)}
          onSave={savePreset}
        />
      )}
    </section>
  );
}
