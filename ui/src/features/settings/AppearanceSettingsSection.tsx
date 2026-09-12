import type { AppearancePreferences, UiDensity } from "../../ipc/bindings";
import { ipc } from "../../ipc/client";
import { useAppStore } from "../../store/store";
import { SettingRow, SettingsSwitch } from "./SettingControls";
import { ThemePicker } from "./ThemePicker";
import { useTranslation } from "../../i18n/useTranslation";
import { DEFAULT_VAULT_PAGE_SIZE } from "../../shared/browsingPreferences";

const save = (preferences: AppearancePreferences) =>
  ipc.send({ kind: "Settings", command: { type: "setAppearance", payload: { preferences } } });

/** Within `MIN_UI_SCALE`/`MAX_UI_SCALE` in the domain, which clamps anything else. */
const UI_SCALES = [100, 125, 150, 175] as const;

/**
 * The named sidebar widths.
 *
 * The panel has been draggable for a while and the width is remembered, which
 * left no way back to the default once it had been dragged: a setting is where
 * people look for that. 224 is the width the client has always opened at and
 * stays the default; 64 is the icon rail, the same layout the shell draws for
 * a narrow window.
 *
 * Dragging still works and is not restricted to these three. A width in
 * between simply lights none of them up, which is the truth.
 */
const SIDEBAR_WIDTH_OPTIONS = [
  { value: 64, labelKey: "settings.appearance.sidebarIcons" as const },
  { value: 224, labelKey: "settings.appearance.sidebarNormal" as const },
  { value: 300, labelKey: "settings.appearance.sidebarWide" as const },
] as const;

const TILE_COLUMN_OPTIONS = [
  { value: 0, labelKey: "settings.appearance.tileColumnsAuto" as const },
  { value: 1, label: "1" },
  { value: 2, label: "2" },
  { value: 3, label: "3" },
  { value: 4, label: "4" },
  { value: 5, label: "5" },
  { value: 6, label: "6" },
] as const;

/**
 * Entries per page in the four vault lists.
 *
 * One setting rather than four, because the question is how much of the screen
 * a reader wants filled, and answering it separately for maps and mods and for
 * installed and vault would be four controls saying the same thing. 36 is what
 * the lists have always opened at and stays the default.
 */
const VAULT_PAGE_SIZE_OPTIONS = [24, 36, 60, 96, 150] as const;

export function AppearanceSettingsSection() {
  const { t } = useTranslation();
  const preferences = useAppStore((state) => state.state.settings.appearance);
  const browsing = useAppStore((state) => state.state.settings.browsing);

  const saveVaultPageSize = (size: number) =>
    ipc.send({
      kind: "Settings",
      command: {
        type: "setBrowsing",
        payload: {
          preferences: {
            ...browsing,
            vaultPageSize: size === DEFAULT_VAULT_PAGE_SIZE ? 0 : size,
          },
        },
      },
    });
  const activePageSize = browsing.vaultPageSize || DEFAULT_VAULT_PAGE_SIZE;

  return (
    <>
      <div className="setting-block">
        <span className="setting-label">{t("settings.appearance.theme")}</span>
        <span className="muted">{t("settings.appearance.themeHint")}</span>
        <ThemePicker />
      </div>
      <SettingRow label={t("settings.appearance.interfaceDensity")} hint={t("settings.appearance.interfaceDensityHint")}>
        <div className="settings-segmented surface" role="group" aria-label={t("settings.appearance.interfaceDensity")}>
          {(["compact", "comfortable"] as UiDensity[]).map((density) => (
            <button
              type="button"
              key={density}
              className={preferences.density === density ? "is-active" : ""}
              aria-pressed={preferences.density === density}
              onClick={() => void save({ ...preferences, density })}
            >
              {t(density === "compact" ? "settings.appearance.compact" : "settings.appearance.comfortable")}
            </button>
          ))}
        </div>
      </SettingRow>
      <SettingRow
        label={t("settings.appearance.interfaceScale")}
        hint={t("settings.appearance.interfaceScaleHint")}
      >
        <div className="settings-segmented surface" role="group" aria-label={t("settings.appearance.interfaceScale")}>
          {UI_SCALES.map((scale) => (
            <button
              type="button"
              key={scale}
              className={preferences.uiScale === scale ? "is-active" : ""}
              aria-pressed={preferences.uiScale === scale}
              onClick={() => void save({ ...preferences, uiScale: scale })}
            >
              {scale}%
            </button>
          ))}
        </div>
      </SettingRow>
      <SettingRow
        label={t("settings.appearance.tileColumns")}
        hint={t("settings.appearance.tileColumnsHint")}
      >
        <div className="settings-segmented surface" role="group" aria-label={t("settings.appearance.tileColumns")}>
          {TILE_COLUMN_OPTIONS.map((option) => {
            const isActive = (preferences.gameTileColumns ?? 0) === option.value;
            return (
              <button
                type="button"
                key={option.value}
                className={isActive ? "is-active" : ""}
                aria-pressed={isActive}
                onClick={() => void save({ ...preferences, gameTileColumns: option.value })}
              >
                {"labelKey" in option ? t(option.labelKey) : option.label}
              </button>
            );
          })}
        </div>
      </SettingRow>
      <SettingRow
        label={t("settings.appearance.vaultPageSize")}
        hint={t("settings.appearance.vaultPageSizeHint")}
      >
        <div
          className="settings-segmented surface"
          role="group"
          aria-label={t("settings.appearance.vaultPageSize")}
        >
          {VAULT_PAGE_SIZE_OPTIONS.map((size) => (
            <button
              type="button"
              key={size}
              className={activePageSize === size ? "is-active" : ""}
              aria-pressed={activePageSize === size}
              onClick={() => void saveVaultPageSize(size)}
            >
              {size}
            </button>
          ))}
        </div>
      </SettingRow>
      <SettingRow
        label={t("settings.appearance.sidebarWidth")}
        hint={t("settings.appearance.sidebarWidthHint")}
      >
        <div className="settings-segmented surface" role="group" aria-label={t("settings.appearance.sidebarWidth")}>
          {SIDEBAR_WIDTH_OPTIONS.map((option) => {
            const isActive = preferences.sidebarWidth === option.value;
            return (
              <button
                type="button"
                key={option.value}
                className={isActive ? "is-active" : ""}
                aria-pressed={isActive}
                onClick={() => void save({ ...preferences, sidebarWidth: option.value })}
              >
                {t(option.labelKey)}
              </button>
            );
          })}
        </div>
      </SettingRow>
      <SettingRow label={t("settings.appearance.reduceMotion")} hint={t("settings.appearance.reduceMotionHint")}>
        <SettingsSwitch
          checked={preferences.reduceMotion}
          onChange={(reduceMotion) => void save({ ...preferences, reduceMotion })}
          label={t("settings.appearance.reduceMotion")}
        />
      </SettingRow>
    </>
  );
}
