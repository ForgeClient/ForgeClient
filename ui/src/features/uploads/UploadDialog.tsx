// Publishing a local map or mod to the vault.
//
// Mirrors the reference clients' upload widgets: confirm what is being
// published, set the ranked flag (maps only: mods have no equivalent in
// either client), then watch the stages go by.
//
// The confirmation used to be a name and a folder, and it said the same
// sentence whether the vault had never heard of the thing or already held four
// versions of it. An author updating their own mod was told they were
// uploading a new one, which is alarming in exactly the situation where being
// wrong is expensive. So this screen now says which of the two is happening,
// shows the fields an author can check it against, and takes the agreement to
// the vault rules here rather than before the file picker: the rules are about
// what is being published, and nobody knows that yet at the picker.

import { useEffect, useState } from "react";

import { Button } from "../../design-system/Button";
import { Icon } from "../../design-system/Icon";
import { Modal } from "../../design-system/Modal";
import type { UploadKind, UploadsState } from "../../ipc/bindings";
import { ipc } from "../../ipc/client";
import { native } from "../../ipc/native";
import { isUploadBusy } from "../../store/reducers/uploads";
import { useAppStore } from "../../store/store";
import { MapThumbnail } from "../../shared/MapThumbnail";
import { formatBytes } from "../../shared/formatBytes";
import { openHttpsUrl } from "../../shared/externalLinks";
import { uploadDescription, uploadFacts, vaultPresence } from "./uploadSubject";
import "./uploads.css";
import { t } from "../../i18n";
import type { MessageKey } from "../../i18n/catalog/en";
import { useLocale } from "../../i18n/useTranslation";

/** The vault rules, which an uploader is agreeing to. */
export const VAULT_RULES_URL = "https://wiki.faforever.com/en/Development/Vault/Rules";

export const openUpload = (kind: UploadKind, folderName: string, displayName: string) =>
  ipc.send({
    kind: "Uploads",
    command: {
      type: "open",
      payload: {
        request: { kind, folderName, displayName, ranked: false, sourcePath: null, renameTo: "" },
      },
    },
  });

/**
 * Where the picker should open for this kind of upload.
 *
 * `undefined` when the backend cannot resolve or create the folder, which is
 * the same as not asking: the dialog opens at its own default and the upload
 * still works. A misconfigured path is not a reason to refuse to publish.
 */
async function defaultUploadDir(kind: UploadKind): Promise<string | undefined> {
  try {
    return await native.clientFolderPath(kind === "map" ? "maps" : "mods");
  } catch {
    return undefined;
  }
}

/** The last path segment, for either separator: Windows gives back backslashes. */
const folderNameOf = (path: string): string =>
  path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? "";

/**
 * Publish a folder picked from disk rather than one the client installed.
 *
 * Java's equivalent entry point is `MapUploadController.setMapPath`, reached
 * from the vault rather than from the installed list: a map being published is
 * usually one the author just built, which by definition is not in the vault
 * yet. The backend does the real validation; this only refuses a path with no
 * usable last segment, which a picker should never return.
 */
export async function openUploadFromDisk(kind: UploadKind): Promise<void> {
  const path = await native.selectFile({
    directory: true,
    title: t(kind === "map" ? "uploads.pick.map" : "uploads.pick.mod"),
    // Start in the folder the thing being published actually lives in.
    //
    // Without this the dialog opens wherever the OS last left it, which on
    // Windows is Documents: two levels above the maps and mods directories,
    // and with no hint that the client already knows where they are.
    defaultPath: await defaultUploadDir(kind),
  });
  if (path === null) return;
  const folderName = folderNameOf(path);
  if (folderName === "") return;
  ipc.send({
    kind: "Uploads",
    command: {
      type: "open",
      payload: {
        request: {
          kind,
          folderName,
          displayName: folderName,
          ranked: false,
          sourcePath: path,
          renameTo: "",
        },
      },
    },
  });
}

const close = () => ipc.send({ kind: "Uploads", command: { type: "close" } });
const setRanked = (ranked: boolean) =>
  ipc.send({ kind: "Uploads", command: { type: "setRanked", payload: { ranked } } });
const start = () => ipc.send({ kind: "Uploads", command: { type: "start" } });

function statusLine(status: UploadsState["status"]): string | null {
  switch (status.type) {
    case "idle":
      return null;
    case "compressing":
      return t("uploads.compressing");
    case "uploading": {
      const { sentBytes, totalBytes } = status.payload;
      const mb = (totalBytes / (1024 * 1024)).toFixed(1);
      return sentBytes >= totalBytes && totalBytes > 0
        ? t("uploads.uploaded", { mb })
        : t("uploads.uploading", { mb });
    }
    case "finishing":
      return t("uploads.registering");
    case "succeeded":
      return null;
    case "failed":
      return status.payload.reason;
  }
}

/**
 * How far along, as a percentage, or `null` when nothing measurable is moving.
 *
 * Mirrors `UploadStatus::percent` rather than reading a second field across
 * the boundary, which is the same reason the Rust side has it: one rule, two
 * renderings of it.
 */
function percentOf(status: UploadsState["status"]): number | null {
  if (status.type === "compressing") {
    const { doneBytes, totalBytes } = status.payload;
    return totalBytes > 0 ? Math.min(100, Math.floor((doneBytes / totalBytes) * 100)) : null;
  }
  if (status.type === "uploading") {
    const { sentBytes, totalBytes } = status.payload;
    return totalBytes > 0 ? Math.min(100, Math.floor((sentBytes / totalBytes) * 100)) : null;
  }
  return null;
}

export function UploadDialog() {
  useLocale();
  const { request, status, preview } = useAppStore((store) => store.state.uploads);
  const mods = useAppStore((store) => store.state.mods);
  const maps = useAppStore((store) => store.state.maps);
  const [accepted, setAccepted] = useState(false);
  // The sentence around the link. Split on a placeholder rather than assembled
  // from fragments, so a translation is free to put the rules where its own
  // grammar wants them.
  const [acceptBefore, acceptAfter] = t("uploads.rules.accept", { rules: "\u0000" }).split("\u0000");

  const kind = request?.kind;
  const folderName = request?.folderName;

  // A fresh subject is a fresh agreement. Carrying the tick over from the last
  // publish would make the checkbox a formality, which is the opposite of why
  // it moved here.
  useEffect(() => setAccepted(false), [kind, folderName]);

  // The catalogue is what decides new-versus-update, and it is a load-once
  // cache every other view primes the same way. Asking for it here covers the
  // publish that is reached before either vault tab has been opened.
  const modsIdle = mods.vaultStatus.type === "idle";
  const mapsIdle = maps.vaultStatus.type === "idle";
  useEffect(() => {
    if (kind === "mod" && modsIdle) ipc.send({ kind: "Mods", command: { type: "loadVault" } });
    if (kind === "map" && mapsIdle) ipc.send({ kind: "Maps", command: { type: "loadVault" } });
  }, [kind, modsIdle, mapsIdle]);

  if (request === null) return null;

  const isMap = request.kind === "map";
  const busy = isUploadBusy(status);
  const done = status.type === "succeeded";
  const line = statusLine(status);
  const percent = percentOf(status);

  const installed = isMap
    ? maps.installed.find((map) => map.folderName === request.folderName)
    : mods.installed.find((mod) => mod.folderName === request.folderName);

  // Only known once compression has measured the folder, which is the first
  // moment anybody knows it: the archive does not exist before then.
  const folderBytes =
    status.type === "compressing"
      ? status.payload.totalBytes
      : status.type === "uploading"
        ? status.payload.totalBytes
        : null;

  const presence = isMap
    ? vaultPresence(request, maps.vault, maps.vaultStatus.type === "ready")
    : vaultPresence(request, mods.vault, mods.vaultStatus.type === "ready");

  const facts = uploadFacts(request, installed, folderBytes, formatBytes);
  const description = uploadDescription(installed);

  return (
    <Modal className="upload-dialog" onClose={close}>
      <header className="upload-dialog-header">
        <h2>{t(isMap ? "uploads.title.map" : "uploads.title.mod")}</h2>
        {/* The sentence Nuggets was owed. "Update" is not a guess: it means
            the vault already holds an entry under this exact name, and FAF
            files every upload under that name as another version of it. */}
        {presence !== "unknown" && (
          <span className={`upload-badge is-${presence}`}>
            {t(
              presence === "update"
                ? isMap
                  ? "uploads.badge.updateMap"
                  : "uploads.badge.updateMod"
                : isMap
                  ? "uploads.badge.newMap"
                  : "uploads.badge.newMod",
            )}
          </span>
        )}
      </header>

      <p className="muted upload-lede">
        {t(
          presence === "update"
            ? "uploads.descriptionUpdate"
            : presence === "new"
              ? "uploads.descriptionNew"
              : "uploads.description",
          { name: request.displayName },
        )}
      </p>

      <div className="upload-subject">
        {/* The map's own picture, read out of its `.scmap`, and only then the
            vault's. A map being published for the first time has no vault
            entry, so `MapThumbnail` has nothing to find and drew a placeholder
            for the one case this dialog exists to serve. The file on disk is
            the only likeness of it that exists yet. */}
        {isMap && preview !== "" && (
          <img className="upload-preview" src={preview} alt={request.displayName} />
        )}
        {isMap && preview === "" && (
          <MapThumbnail
            mapName={request.folderName}
            vault={maps.vault}
            className="upload-preview"
            placeholderClassName="upload-preview is-placeholder"
            large
          />
        )}
        <dl className="upload-facts">
          {facts.map((fact) => (
            <div key={fact.labelKey}>
              <dt>{t(fact.labelKey as MessageKey)}</dt>
              <dd className={fact.labelKey === "uploads.fact.folder" ? "upload-folder" : undefined}>
                {fact.value}
              </dd>
            </div>
          ))}
        </dl>
      </div>

      {description && <p className="upload-description muted">{description}</p>}

      {/* A rename is not a rename as far as FAF is concerned: it is a new mod
          with a new uid, and this is the last screen before that happens. */}
      {request.renameTo !== "" && (
        <p className="upload-status muted">
          {t("uploads.renamingTo", { name: request.renameTo })}
        </p>
      )}

      {/* The rules belong to what is being published, and until this screen
          nothing had been chosen yet. Accepting them at the file picker was
          agreeing to terms about a folder nobody had picked. */}
      {!done && (
        <div className="upload-rules">
          {/* Maps only: the ranked flag decides whether games on it affect
              ratings. Neither reference client offers an equivalent for mods.

              Here rather than above the divider, because it belongs with the
              rules: both are decisions about how this upload will behave once
              it is public, and both are the last things settled before the
              button. */}
          {isMap && (
            <label className="check-field">
              <input
                type="checkbox"
                checked={request.ranked}
                disabled={busy || done}
                onChange={(event) => setRanked(event.target.checked)}
              />
              {t("uploads.allowRanked")}
            </label>
          )}
          {/* One sentence with the rules in it, rather than a link above a
              tick box repeating the same three words. The link is the words
              themselves, which is where a reader goes looking for it. */}
          <label className="check-field upload-rules-accept">
            <input
              type="checkbox"
              checked={accepted}
              disabled={busy}
              onChange={(event) => setAccepted(event.target.checked)}
            />
            <span>
              {acceptBefore}
              <button
                type="button"
                className="upload-rules-link"
                onClick={(event) => {
                  // Inside the label, so without this the link would also tick
                  // the box it sits in.
                  event.preventDefault();
                  event.stopPropagation();
                  ipc.run(openHttpsUrl(VAULT_RULES_URL));
                }}
              >
                {t("uploads.rules.word")}
                <Icon name="external" size={12} />
              </button>
              {acceptAfter}
            </span>
          </label>
        </div>
      )}

      {percent !== null && (
        <div
          className="upload-progress"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={percent}
          aria-label={t(status.type === "compressing" ? "uploads.compressing" : "uploads.progress")}
        >
          <i style={{ width: `${percent}%` }} />
        </div>
      )}

      {done && <p className="upload-status is-ok">{t("uploads.published")}</p>}
      {line && (
        <p className={status.type === "failed" ? "upload-status is-error" : "upload-status muted"}>
          {line}
          {percent !== null && ` ${percent}%`}
        </p>
      )}

      <div className="upload-actions">
        <Button onClick={close}>{t(done ? "uploads.close" : "uploads.cancel")}</Button>
        {!done && (
          <Button
            variant="primary"
            disabled={busy || !accepted}
            title={accepted ? undefined : t("uploads.acceptFirst")}
            onClick={start}
          >
            {t(busy ? "uploads.publishing" : "uploads.publish")}
          </Button>
        )}
      </div>
    </Modal>
  );
}
