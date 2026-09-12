import { useEffect, useMemo, useState } from "react";
import { Button } from "../../design-system/Button";
import { Modal } from "../../design-system/Modal";
import { ipc } from "../../ipc/client";
import { useAppStore } from "../../store/store";
import { formatDateTime } from "../../shared/dates";
import "./reporting.css";
import { useTranslation } from "../../i18n/useTranslation";

const close = () => ipc.send({ kind: "Reporting", command: { type: "close" } });

export function ReportPlayerModal() {
  const { t } = useTranslation();
  const report = useAppStore((state) => state.state.reporting);
  const [description, setDescription] = useState("");
  const [gameId, setGameId] = useState("");
  const [incidentTime, setIncidentTime] = useState("");
  const [view, setView] = useState<"new" | "history">("new");

  useEffect(() => {
    if (!report.open) return;
    setDescription("");
    setGameId("");
    setIncidentTime("");
    setView("new");
  }, [report.login, report.open, report.playerId]);

  const parsedGameId = gameId.trim() ? Number(gameId) : null;
  const validation = useMemo(() => {
    const length = description.trim().length;
    if (length > 0 && length < 10) return t("reporting.error.tooShort");
    if (length > 4_000) return t("reporting.error.tooLong");
    if (gameId.trim() && (!Number.isInteger(parsedGameId) || (parsedGameId ?? 0) <= 0)) {
      return t("reporting.error.gameId");
    }
    if (parsedGameId !== null && !incidentTime.trim()) {
      return t("reporting.error.gameTime");
    }
    return "";
  }, [description, gameId, incidentTime, parsedGameId, t]);

  if (!report.open || report.playerId === null) return null;
  const submitting = report.status.type === "submitting";
  const submitted = report.status.type === "submitted";
  const failure = report.status.type === "failed" ? report.status.payload.reason : "";
  const canSubmit = description.trim().length >= 10 && !validation && !submitting && !submitted;

  const submit = () => {
    if (!canSubmit || report.playerId === null) return;
    ipc.send({
      kind: "Reporting",
      command: {
        type: "submit",
        payload: {
          playerId: report.playerId,
          login: report.login,
          description: description.trim(),
          gameId: parsedGameId,
          incidentTime: incidentTime.trim(),
        },
      },
    });
  };

  return (
    <Modal onClose={() => { if (!submitting) void close(); }} className="report-player-modal">
      <form onSubmit={(event) => { event.preventDefault(); submit(); }}>
        <header className="report-player-head">
          <span className="report-player-eyebrow">{t("reporting.title")}</span>
          <h2>{t("reporting.reportPlayer", { name: report.login })}</h2>
          <p className="muted">
            {t("reporting.intro")}
          </p>
        </header>

        <div className="report-tabs" role="tablist" aria-label={t("reporting.reportingViews")}>
          <button type="button" role="tab" aria-selected={view === "new"} className={view === "new" ? "active" : ""} onClick={() => setView("new")}>{t("reporting.tab.new")}</button>
          <button type="button" role="tab" aria-selected={view === "history"} className={view === "history" ? "active" : ""} onClick={() => setView("history")}>{t("reporting.tab.history")} <span>{report.history.length}</span></button>
        </div>

        {view === "history" ? (
          <section className="report-history" role="tabpanel">
            {report.historyStatus.type === "loading" && <p className="muted" role="status">Loading your reports…</p>}
            {report.historyStatus.type === "failed" && (
              <div className="report-history-error" role="alert">
                <span>{report.historyStatus.payload.reason}</span>
                <Button type="button" onClick={() => ipc.send({ kind: "Reporting", command: { type: "loadHistory" } })}>{t("common.retry")}</Button>
              </div>
            )}
            {report.historyStatus.type === "ready" && report.history.length === 0 && <p className="muted">{t("reporting.historyEmpty")}</p>}
            {report.history.map((item) => (
              <article className="report-history-card surface" key={item.id}>
                <header>
                  <div><strong>Report #{item.id}</strong><time dateTime={item.createTime}>{formatDateTime(item.createTime)}</time></div>
                  <span className="report-history-status">{item.status || t("reporting.statusFallback")}</span>
                </header>
                <dl>
                  <div><dt>{t("reporting.offender")}</dt><dd>{item.offenders.join(", ") || t("common.unknown")}</dd></div>
                  <div><dt>{t("reporting.game")}</dt><dd>{item.gameId ? `#${item.gameId}` : t("reporting.notGameRelated")}</dd></div>
                  <div><dt>{t("reporting.moderator")}</dt><dd>{item.moderator || t("reporting.unassigned")}</dd></div>
                </dl>
                <p>{item.description}</p>
                {item.moderatorNotice && <aside><strong>{t("reporting.moderatorNotice")}</strong><span>{item.moderatorNotice}</span></aside>}
              </article>
            ))}
          </section>
        ) : submitted ? (
          <div className="report-success" role="status">
            <strong>{t("reporting.submitted")}</strong>
            <span>{t("reporting.submittedHint")}</span>
          </div>
        ) : (
          <>
            <label className="report-field">
              <span>{t("reporting.whatHappened")} <em>{t("reporting.required")}</em></span>
              <textarea
                autoFocus
                value={description}
                maxLength={4_000}
                rows={7}
                disabled={submitting}
                onChange={(event) => setDescription(event.target.value)}
                placeholder={t("reporting.describeBehaviorContext")}
              />
              <small>{description.length} / 4,000 characters</small>
            </label>
            <div className="report-game-fields">
              <label className="report-field">
                <span>{t("reporting.gameId")} <em>{t("reporting.optional")}</em></span>
                <input
                  type="number"
                  min={1}
                  step={1}
                  value={gameId}
                  disabled={submitting}
                  onChange={(event) => setGameId(event.target.value)}
                  placeholder={t("reporting.eG12345678")}
                />
              </label>
              <label className="report-field">
                <span>{t("reporting.gameTime")} {gameId.trim() ? <em>{t("reporting.required")}</em> : <em>{t("reporting.optional")}</em>}</span>
                <input
                  value={incidentTime}
                  disabled={submitting}
                  onChange={(event) => setIncidentTime(event.target.value)}
                  placeholder={t("reporting.eG18")}
                />
              </label>
            </div>
            {(validation || failure) && <p className="report-error" role="alert">{failure || validation}</p>}
          </>
        )}

        <footer className="report-actions">
          <Button type="button" onClick={() => void close()} disabled={submitting}>
            {t(submitted ? "common.close" : "common.cancel")}
          </Button>
          {view === "new" && !submitted && <Button type="submit" variant="primary" disabled={!canSubmit}>
            {t(submitting ? "reporting.submitting" : "reporting.submit")}
          </Button>}
        </footer>
      </form>
    </Modal>
  );
}
