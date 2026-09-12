import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { SocialState, VaultMap } from "../../ipc/bindings";
import { ipc } from "../../ipc/client";
import { MapThumbnail } from "../../shared/MapThumbnail";
import { mapPresentation } from "../../shared/mapPresentation";
import { type GamePresence } from "./gameSummary";
import { GameSummaryCard, STATUS_LABEL } from "./GameSummaryCard";
import { GameStatusSword } from "./GameStatusSword";
import { useTranslation } from "../../i18n/useTranslation";
import { joinGame } from "../lobby/joinGame";

interface Props {
  presence: GamePresence;
  social: SocialState;
  vault: VaultMap[];
}

export function GameSummaryPopover({ presence, social, vault }: Props) {
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState({ top: 8, right: 8 });
  const anchor = useRef<HTMLButtonElement>(null);
  const tooltipId = useId();
  const presentation = mapPresentation(vault, presence.game.map);

  const updatePosition = useCallback(() => {
    const rect = anchor.current?.getBoundingClientRect();
    if (!rect) return;
    const viewportWidth = document.documentElement.clientWidth || window.innerWidth;
    const viewportHeight = document.documentElement.clientHeight || window.innerHeight;
    setPosition({
      top: Math.max(8, Math.min(rect.top, viewportHeight - 280)),
      right: Math.max(8, viewportWidth - rect.left + 8),
    });
  }, []);

  useLayoutEffect(() => {
    if (!open) return;
    updatePosition();
    const handleClose = () => setOpen(false);
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    window.addEventListener("blur", handleClose);
    document.addEventListener("visibilitychange", handleClose);
    return () => {
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
      window.removeEventListener("blur", handleClose);
      document.removeEventListener("visibilitychange", handleClose);
    };
  }, [open, updatePosition]);

  // Only the one card currently visible needs a clock, and only while it is
  // open: a several-hundred-user channel must not run a timer per badge.
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  useEffect(() => {
    if (!open) return;
    setNow(Math.floor(Date.now() / 1000));
    const timer = window.setInterval(() => setNow(Math.floor(Date.now() / 1000)), 30_000);
    return () => window.clearInterval(timer);
  }, [open]);

  const { t } = useTranslation();
  const status = t(STATUS_LABEL[presence.status]);

  const handleDoubleClick = (event: React.MouseEvent) => {
    event.stopPropagation();
    event.preventDefault();
    if (presence.status === "playing" || presence.status === "playingDelayed") {
      ipc.send({
        kind: "Replays",
        command: {
          type: "watchLive",
          payload: {
            uid: presence.game.id,
            modName: presence.game.modName,
            map: presence.game.map,
          },
        },
      });
    } else if (presence.status === "hosting" || presence.status === "lobbying") {
      void joinGame(presence.game.id);
    }
  };

  return (
    <>
      <button
        ref={anchor}
        type="button"
        className="chat-game-badge"
        aria-label={t("chat.gameBadge.aria", {
          status,
          title: presence.game.title,
          map: presentation.displayName,
        })}
        aria-describedby={open ? tooltipId : undefined}
        onMouseEnter={() => setOpen(true)}
        onMouseLeave={() => setOpen(false)}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        onDoubleClick={handleDoubleClick}
      >
        <GameStatusSword status={presence.status} />
        <MapThumbnail
          mapName={presence.game.map}
          vault={vault}
          className="chat-game-map"
          placeholderClassName="chat-game-map chat-game-map-placeholder"
          preferCanonicalPreview
        />
      </button>
      {open && createPortal(
        <aside
          id={tooltipId}
          role="tooltip"
          className="chat-game-popover"
          style={position}
        >
          <GameSummaryCard presence={presence} social={social} vault={vault} now={now} />
        </aside>,
        document.body,
      )}
    </>
  );
}
