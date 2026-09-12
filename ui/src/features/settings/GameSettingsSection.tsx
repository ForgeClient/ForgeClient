import { useEffect, useState } from "react";
import type { GamePreferences } from "../../ipc/bindings";
import { ipc } from "../../ipc/client";
import { Button } from "../../design-system/Button";
import { NumberInput } from "../../design-system/NumberInput";
import { useAppStore } from "../../store/store";
import { useTranslation } from "../../i18n/useTranslation";
import { SettingRow, SettingsSwitch } from "./SettingControls";
import { gameNeedsALaunchWrapper } from "../../shared/platform";

/// Mirrors `faf_domain::state::settings::MAX_KEPT_GENERATED_MAPS`, so the
/// input never offers a number the backend would quietly clamp.
const MAX_KEPT_GENERATED_MAPS = 500;

const save = (preferences: GamePreferences) =>
  ipc.send({ kind: "Settings", command: { type: "setGame", payload: { preferences } } });

export function GameSettingsSection() {
  const { t } = useTranslation();
  const preferences = useAppStore((state) => state.state.settings.game);
  const rawArguments = preferences.additionalArguments ?? [];
  const [argumentsText, setArgumentsText] = useState(rawArguments.join("\n"));
  const persistedText = rawArguments.join("\n");

  useEffect(() => setArgumentsText(persistedText), [persistedText]);

  const commitArguments = () => {
    if (argumentsText === persistedText) return;
    void save({
      ...preferences,
      additionalArguments: argumentsText.split(/\r?\n/).map((argument) => argument.trim()).filter(Boolean),
    });
  };

  const setAutoGenerate = (autoGenerateMaps: boolean) => {
    void save({ ...preferences, autoGenerateMaps });
  };

  const setConfirmDownloads = (confirmDownloadsBeforeJoining: boolean) => {
    void save({ ...preferences, confirmDownloadsBeforeJoining });
  };

  const [wrapperText, setWrapperText] = useState(preferences.launchWrapper ?? "");
  const persistedWrapper = preferences.launchWrapper ?? "";
  useEffect(() => setWrapperText(persistedWrapper), [persistedWrapper]);
  const commitWrapper = () => {
    if (wrapperText.trim() === persistedWrapper) return;
    void save({ ...preferences, launchWrapper: wrapperText.trim() });
  };

  const setPipeLiveReplay = (pipeLiveReplay: boolean) => {
    void save({ ...preferences, pipeLiveReplay });
  };

  const setKeepGeneratedMaps = (keepGeneratedMaps: boolean) => {
    void save({ ...preferences, keepGeneratedMaps });
  };

  const setKeepGeneratedMapsLimit = (keepGeneratedMapsLimit: number) => {
    void save({
      ...preferences,
      keepGeneratedMapsLimit: Math.max(0, Math.min(MAX_KEPT_GENERATED_MAPS, keepGeneratedMapsLimit)),
    });
  };

  return (
    <>
      {/* First, because it is the one switch here that changes what a
          double-click does. The dialog it controls carries its own "do not ask
          again", which is what most people will use to turn it off; this row
          is how they turn it back on. */}
      <SettingRow
        label={t("settings.game.confirmDownloads")}
        hint={t("settings.game.confirmDownloadsHint")}
      >
        <SettingsSwitch
          checked={preferences.confirmDownloadsBeforeJoining ?? true}
          onChange={setConfirmDownloads}
          label={t("settings.game.confirmDownloads")}
        />
      </SettingRow>

      <SettingRow
        label={t("settings.game.autoGenerateMaps")}
        hint={t("settings.game.autoGenerateMapsHint")}
      >
        <SettingsSwitch
          checked={preferences.autoGenerateMaps ?? true}
          onChange={setAutoGenerate}
          label={t("settings.game.autoGenerateMaps")}
        />
      </SettingRow>

      {/* Beside the switch that decides when maps are generated, because this
          one decides what happens to them afterwards. It used to be a checkbox
          inside the Generate map dialog, answered once per run at the moment
          nobody has seen the maps yet. */}
      <SettingRow
        label={t("settings.game.keepGeneratedMaps")}
        hint={t("settings.game.keepGeneratedMapsHint")}
      >
        <div className="settings-inline-pair">
          <SettingsSwitch
            checked={preferences.keepGeneratedMaps ?? false}
            onChange={setKeepGeneratedMaps}
            label={t("settings.game.keepGeneratedMaps")}
          />
          {/* How many, next to whether. Keeping everything is what filled a
              system drive in the thread that asked for this, and keeping
              nothing is already the switch beside it; zero means no limit
              because that is the old behaviour and it stays reachable. */}
          <NumberInput
            className="number-input"
            value={preferences.keepGeneratedMapsLimit ?? 0}
            min={0}
            max={MAX_KEPT_GENERATED_MAPS}
            disabled={!(preferences.keepGeneratedMaps ?? false)}
            aria-label={t("settings.game.keepGeneratedMapsLimit")}
            title={t("settings.game.keepGeneratedMapsLimitHint")}
            onChange={setKeepGeneratedMapsLimit}
          />
        </div>
      </SettingRow>

      <SettingRow
        label={t("settings.game.pipeLiveReplay")}
        hint={t("settings.game.pipeLiveReplayHint")}
      >
        <SettingsSwitch
          checked={preferences.pipeLiveReplay ?? false}
          onChange={setPipeLiveReplay}
          label={t("settings.game.pipeLiveReplay")}
        />
      </SettingRow>

      {/* Above the arguments, because it decides what runs them. Only where
          the game is not a native binary: on Windows there is nothing to wrap
          and an empty field labelled "launch command" invites somebody to
          fill it in. */}
      {gameNeedsALaunchWrapper() && (
        <div className="setting-block">
          <span className="setting-label">{t("settings.game.launchWrapperLabel")}</span>
          <span className="muted">{t("settings.game.launchWrapperHint")}</span>
          <input
            className="settings-input"
            type="text"
            value={wrapperText}
            onChange={(event) => setWrapperText(event.target.value)}
            onBlur={commitWrapper}
            placeholder="wine"
            aria-label={t("settings.game.launchWrapperLabel")}
          />
          <div className="settings-save-line">
            <span className="muted">{t("settings.game.launchWrapperNote")}</span>
            <Button onClick={commitWrapper} disabled={wrapperText.trim() === persistedWrapper}>
              {t("settings.game.saveArguments")}
            </Button>
          </div>
        </div>
      )}

      <div className="setting-block">
        <span className="setting-label">{t("settings.game.argumentsLabel")}</span>
        <span className="muted">
          {t("settings.game.argumentsHint")}
        </span>
        <textarea
          className="settings-textarea"
          value={argumentsText}
          onChange={(event) => setArgumentsText(event.target.value)}
          onBlur={commitArguments}
          rows={4}
          placeholder={"/windowed\n/size\n1920\n1080"}
          aria-label={t("settings.game.additionalGameLaunch")}
        />
        <div className="settings-save-line">
          <span className="muted">{t("settings.game.argumentsNote")}</span>
          <Button onClick={commitArguments} disabled={argumentsText === persistedText}>{t("settings.game.saveArguments")}</Button>
        </div>
      </div>
    </>
  );
}
