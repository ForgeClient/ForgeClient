import { useState } from "react";
import { Button } from "../../design-system/Button";
import { Modal } from "../../design-system/Modal";
import { native, type LogIssue, type LogKind, type LogPreview } from "../../ipc/native";
import { openExternalUrl } from "../../shared/externalLinks";

/**
 * Where an issue's "More information" goes, empty when there is nothing useful
 * to link to. The URL is data about the issue, but it lives here rather than in
 * the domain because `externalLinks` owns the host allowlist that vets it: this
 * thread is on `forum.faforever.com`, which that allowlist already covers.
 *
 * Same target as the Java client's `helpLinks.soundIssues`.
 */
const LOG_ISSUE_HELP: Record<LogIssue, string> = {
  gameMinimized: "",
  soundDriver:
    "https://forum.faforever.com/topic/4084/solutions-for-snd-error-xact-invalid-arg-xact3dapply-failed",
};
import { SettingRow } from "./SettingControls";
import { useTranslation } from "../../i18n/useTranslation";

export function DiagnosticsSettingsSection() {
  const { t } = useTranslation();
  const [preview, setPreview] = useState<LogPreview | null>(null);
  const [error, setError] = useState("");
  const openFolder = (kind: LogKind) => {
    setError("");
    void native.openLogFolder(kind).catch((reason) => setError(String(reason)));
  };
  const viewLatest = (kind: LogKind) => {
    setError("");
    void native.readLatestLog(kind)
      .then((result) => result ? setPreview(result) : setError(t("settings.diagnostics.noLogs")))
      .catch((reason) => setError(String(reason)));
  };

  return (
    <>
      <SettingRow label={t("settings.diagnostics.gameLogs")} hint={t("settings.diagnostics.gameLogsHint")}>
        <div className="settings-diagnostic-actions">
          <Button onClick={() => viewLatest("game")}>{t("settings.diagnostics.viewLatest")}</Button>
          <Button onClick={() => openFolder("game")}>{t("settings.diagnostics.openFolder")}</Button>
        </div>
      </SettingRow>
      <SettingRow label={t("settings.diagnostics.clientLogs")} hint={t("settings.diagnostics.clientLogsHint")}>
        <div className="settings-diagnostic-actions">
          <Button onClick={() => viewLatest("client")}>{t("settings.diagnostics.viewLatest")}</Button>
          <Button onClick={() => openFolder("client")}>{t("settings.diagnostics.openFolder")}</Button>
        </div>
      </SettingRow>
      {error && <p className="settings-inline-error" role="alert">{error}</p>}
      {preview && (
        <Modal className="diagnostic-log-modal" onClose={() => setPreview(null)}>
          <h2>{preview.fileName}</h2>
          {/* Above the log, not buried under it: the whole point is that these
              traces are invisible in thousands of lines unless you already know
              the string to search for. */}
          {preview.issues.length > 0 && (
            <section className="log-analysis" role="status">
              <p className="log-analysis-heading">{t("log.analysis.heading")}</p>
              <ul>
                {preview.issues.map((issue) => (
                  <li key={issue}>
                    <span>{t(`log.analysis.${issue}`)}</span>
                    {LOG_ISSUE_HELP[issue] && (
                      <button
                        type="button"
                        className="log-analysis-link"
                        onClick={() => void openExternalUrl(LOG_ISSUE_HELP[issue])}
                      >
                        {t("log.analysis.moreInfo")}
                      </button>
                    )}
                  </li>
                ))}
              </ul>
            </section>
          )}
          <p className="muted">{t("settings.diagnostics.truncated")}</p>
          <textarea
            readOnly
            value={preview.content}
            aria-label={t("settings.diagnostics.contentsOf", { file: preview.fileName })}
          />
          <div className="settings-diagnostic-actions">
            <Button onClick={() => void navigator.clipboard.writeText(preview.content)}>{t("settings.diagnostics.copy")}</Button>
            <Button variant="primary" onClick={() => setPreview(null)}>{t("settings.diagnostics.close")}</Button>
          </div>
        </Modal>
      )}
    </>
  );
}
