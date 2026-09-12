// The sidebar's Game folders button.
//
// Opening the maps folder used to mean: Settings, the Paths section, scroll
// past nine rows of path configuration, then the row of buttons. That is the
// "doom scrolling" the issue describes, and it is the wrong shape for something
// people do while a game is loading.
//
// It expands in place rather than opening a panel over the client. The list is
// short and it belongs to the sidebar it sits in, and an inline disclosure has
// no anchoring to get wrong when the sidebar is resized or the window is short.
//
// Upward, though. This sits in the bottom-anchored group, so a list rendered
// under the button pushed the button up and out from under the cursor: opening
// it to check something and then closing it again meant chasing it. Above, the
// group grows into the space over it and the button does not move.
// Individual pinning was the original request and was dropped on the thread:
// once the list is one click away there is nothing left to save by choosing
// which half of it to keep.

import { useState } from "react";
import { Icon } from "../../design-system/Icon";
import { useTranslation } from "../../i18n/useTranslation";
import { FOLDER_GROUPS, openFolderEntry, type FolderEntry } from "./gameFolders";
import "./game-folders.css";

export function GameFoldersMenu() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  // The shell's own message, shown under the list rather than swallowed: a
  // folder that is not there yet (no replays played, no vault) is the common
  // case and the reason has to be readable.
  const [error, setError] = useState("");

  const openEntry = (entry: FolderEntry) => {
    setError("");
    void openFolderEntry(entry).catch((reason) => setError(String(reason)));
  };

  return (
    <div className="game-folders">
      {open && (
        <div className="game-folders-list" role="group" aria-label={t("gameFolders.title")}>
          {FOLDER_GROUPS.map((group) => (
            <div className="game-folders-group" key={group.id}>
              <p className="game-folders-group-title">{t(group.title)}</p>
              {group.entries.map((entry) => (
                <button
                  type="button"
                  className="game-folders-entry"
                  key={entry.id}
                  onClick={() => openEntry(entry)}
                >
                  {t(entry.label)}
                </button>
              ))}
            </div>
          ))}
          {error && <p className="game-folders-error" role="alert">{error}</p>}
        </div>
      )}

      <button
        type="button"
        className={open ? "tab game-folders-toggle is-open" : "tab game-folders-toggle"}
        aria-expanded={open}
        title={t("gameFolders.title")}
        onClick={() => setOpen((value) => !value)}
      >
        <Icon name="folder" size={17} />
        <span>{t("gameFolders.title")}</span>
        {/* Points the way the list will move: up to open, down to put away. */}
        <Icon className="game-folders-caret" name={open ? "chevronDown" : "chevronUp"} size={14} />
      </button>
    </div>
  );
}
