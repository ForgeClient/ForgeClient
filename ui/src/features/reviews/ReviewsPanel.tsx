// Vault reviews: read the community's verdict, and write your own.
//
// Mirrors the Java client's `ReviewsController`: an average with a star
// distribution at the top, your own review in an editable block, and
// everyone else's beneath. Opened from a map or mod's detail pane.

import { useEffect, useState } from "react";
import { Button } from "../../design-system/Button";
import { Icon } from "../../design-system/Icon";
import { Modal } from "../../design-system/Modal";
import type { Review, ReviewSummary } from "../../ipc/bindings";
import { ipc } from "../../ipc/client";
import { useAppStore } from "../../store/store";
import { t } from "../../i18n";
import "./reviews.css";
import { useTranslation } from "../../i18n/useTranslation";

const SCORES = [5, 4, 3, 2, 1];

const close = () => ipc.send({ kind: "Reviews", command: { type: "close" } });
const submit = (score: number, text: string) =>
  ipc.send({ kind: "Reviews", command: { type: "submit", payload: { score, text } } });
const remove = () => ipc.send({ kind: "Reviews", command: { type: "delete" } });

/** Twin of `own_review` in the domain: logins do not round-trip case-stably. */
function ownReview(reviews: Review[], login: string): Review | null {
  if (!login) return null;
  return reviews.find((r) => r.player.toLowerCase() === login.toLowerCase()) ?? null;
}

function Stars({ score, of = 5 }: { score: number; of?: number }) {
  const filled = Math.round(score);
  return (
    <span className="review-stars" aria-label={t("reviews.scoreAria", { score: score.toFixed(1), of })}>
      {Array.from({ length: of }, (_, index) => (
        <span key={index} className={index < filled ? "is-filled" : undefined} aria-hidden="true">
          ★
        </span>
      ))}
    </span>
  );
}

export function ReviewsPanel() {
  const { t } = useTranslation();
  const state = useAppStore((store) => store.state.reviews);
  const player = useAppStore((store) => store.state.auth.player);

  if (state.target === null) return null;

  const mine = ownReview(state.reviews, player?.name ?? "");
  const others = state.reviews.filter((review) => review.id !== mine?.id);

  return (
    <Modal className="reviews-modal" onClose={() => void close()}>
      <header className="reviews-head">
        <div>
          <span className="reviews-eyebrow">
            {t(state.target.kind === "map" ? "reviews.mapReviews" : "reviews.modReviews")}
          </span>
          <h2>{state.target.name}</h2>
        </div>
      </header>

      {state.status.type === "loading" && <p className="muted">Loading reviews…</p>}
      {state.status.type === "failed" && (
        <p className="surface-error reviews-error">{state.status.payload.reason}</p>
      )}

      {state.status.type === "ready" && (
        <>
          <Distribution summary={state.summary} />

          {player ? (
            <OwnReview mine={mine} />
          ) : (
            <p className="muted">{t("reviews.signIn")}</p>
          )}

          <section className="reviews-list">
            <h3>
              {others.length === 0
                ? t("reviews.noOthers")
                : t("reviews.otherReviews", { count: others.length })}
            </h3>
            {others.map((review) => (
              <article className="surface review-row" key={review.id}>
                <header>
                  <strong>{review.player || t("reviews.unknownPlayer")}</strong>
                  <Stars score={review.score} />
                  {review.version && (
                    <small className="muted">{t("reviews.version", { version: review.version })}</small>
                  )}
                </header>
                {review.text && <p>{review.text}</p>}
              </article>
            ))}
          </section>
        </>
      )}
    </Modal>
  );
}

function Distribution({ summary }: { summary: ReviewSummary }) {
  return (
    <section className="reviews-summary">
      <div className="reviews-average">
        <strong>{summary.total === 0 ? "N/A" : (summary.averageTenths / 10).toFixed(1)}</strong>
        <Stars score={summary.averageTenths / 10} />
        <small className="muted">
          {summary.total} review{summary.total === 1 ? "" : "s"}
        </small>
      </div>
      <div className="reviews-bars">
        {SCORES.map((score) => {
          const count = summary.counts[score - 1] ?? 0;
          // Guarded the same way both reference clients guard it: an
          // unreviewed subject would otherwise divide by zero.
          const percent = summary.total === 0 ? 0 : (count / summary.total) * 100;
          return (
            <div className="reviews-bar" key={score}>
              <span className="reviews-bar-label">{score}★</span>
              <span className="reviews-bar-track">
                <span className="reviews-bar-fill" style={{ width: `${percent}%` }} />
              </span>
              <span className="reviews-bar-count muted">{count}</span>
            </div>
          );
        })}
      </div>
    </section>
  );
}

function OwnReview({ mine }: { mine: Review | null }) {
  const { t } = useTranslation();
  const submitStatus = useAppStore((store) => store.state.reviews.submit);
  const [score, setScore] = useState(mine?.score ?? 5);
  const [text, setText] = useState(mine?.text ?? "");

  // Adopt the server's version of our review once a write settles, so the
  // editor is not left showing something subtly different from what everyone
  // else can see.
  useEffect(() => {
    setScore(mine?.score ?? 5);
    setText(mine?.text ?? "");
  }, [mine?.id, mine?.score, mine?.text]);

  const saving = submitStatus.type === "saving";

  return (
    <section className="surface reviews-own">
      <h3>{t(mine ? "reviews.yours" : "reviews.write")}</h3>

      <div className="reviews-score-picker" role="group" aria-label={t("reviews.yourScore")}>
        {[1, 2, 3, 4, 5].map((value) => (
          <button
            type="button"
            key={value}
            className={value <= score ? "reviews-score is-on" : "reviews-score"}
            aria-pressed={value === score}
            aria-label={`${value} star${value === 1 ? "" : "s"}`}
            onClick={() => setScore(value)}
          >
            ★
          </button>
        ))}
      </div>

      <textarea
        className="reviews-text"
        value={text}
        maxLength={2000}
        rows={4}
        placeholder={t("reviews.placeholder")}
        onChange={(event) => setText(event.target.value)}
      />

      <div className="reviews-own-actions">
        <Button variant="primary" disabled={saving} onClick={() => void submit(score, text)}>
          {t(saving ? "reviews.saving" : mine ? "reviews.update" : "reviews.post")}
        </Button>
        {mine && (
          <Button disabled={saving} onClick={() => void remove()}>
            <Icon name="close" size={14} /> {t("reviews.withdraw")}
          </Button>
        )}
      </div>

      {submitStatus.type === "failed" && (
        <p className="reviews-submit is-error">{submitStatus.payload.reason}</p>
      )}
      {submitStatus.type === "saved" && <p className="reviews-submit is-ok">{t("reviews.saved")}</p>}
    </section>
  );
}
