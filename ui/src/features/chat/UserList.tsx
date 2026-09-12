// The channel roster.
//
// Takes the Java client's grouped, filterable user list (`ChatUserListController`
// + `ChatUserCategory`): users are bucketed into ordered categories with a
// count per header, and a search box narrows the list without losing the
// grouping. Takes the Python client's "N users (type to filter)" affordance for
// the search field's label, which says what the box does without a tooltip.
//
// Each row carries the same decorations both clients show: avatar, country
// flag and clan tag: all of which come from the lobby's player record rather
// than from IRC, so an IRC-only nickname simply renders bare.
//
// Double-click opens a private conversation; right-click opens the user menu.

import { memo, useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ChatPreferences, ChatUser, Game, PlayerProfile, SocialState, VaultMap } from "../../ipc/bindings";
import { Icon } from "../../design-system/Icon";
import { ipc } from "../../ipc/client";
import { useAppStore } from "../../store/store";
import { isModerator, playersByNickname } from "../../store/reducer";
import { useTranslation } from "../../i18n/useTranslation";
import {
  USER_CATEGORY_LABELS,
  USER_CATEGORY_ORDER,
  displayName,
  resolvedNickStyle,
  type UserCategory,
} from "./chatFormat";
import { flagSrc } from "../../shared/countryFlags";
import { useCountryLabel } from "../../shared/useCountryLabel";
import { GameSummaryPopover } from "./GameSummaryPopover";
import { gamePresenceIndex, type GamePresence } from "./gameSummary";
import { rosterRatingSummary } from "./ratingSummary";

interface Props {
  users: ChatUser[];
  self: string;
  social: SocialState;
  openGames: Game[];
  liveGames: Game[];
  mapVault: VaultMap[];
  preferences: ChatPreferences;
  onOpenConversation: (nick: string) => void;
  onContextMenu: (nick: string, event: React.MouseEvent) => void;
  /**
   * Whose player menu is open, if anybody's.
   *
   * The menu opens under the pointer, which by then has left the row it was
   * opened from, so the row stops looking hovered at the exact moment the
   * menu needs it to say who it is about. The row keeps the hover styling
   * while its menu is up; the menu no longer repeats the name itself.
   */
  menuTarget: string | null;
}

/**
 * Row height, if the stylesheet cannot be read for some reason. Only a fallback:
 * the real value is `--roster-row-height` on `.chat-roster-list`, which is also
 * what makes the rows that tall. Hard-coding it here instead is what broke this
 * list before, because a row is 24 px and a row for someone in a game was 28 px,
 * against an assumed 26 px.
 */
const FALLBACK_ITEM_HEIGHT = 26;
const OVERSCAN = 10; // extra rows to render above and below visible viewport

/** The single row height every roster row is laid out at, from the stylesheet. */
function measureItemHeight(list: HTMLElement): number {
  const declared = getComputedStyle(list).getPropertyValue("--roster-row-height");
  const parsed = Number.parseFloat(declared);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : FALLBACK_ITEM_HEIGHT;
}

type RosterFlatItem =
  | { type: "header"; category: UserCategory; count: number; collapsed: boolean; key: string }
  | { type: "user"; user: ChatUser; category: UserCategory; key: string };

export const UserList = memo(function UserList({
  users,
  self,
  social,
  openGames,
  liveGames,
  mapVault,
  preferences,
  onOpenConversation,
  onContextMenu,
  menuTarget,
}: Props) {
  const { t } = useTranslation();
  const [filter, setFilter] = useState("");
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(600);
  const [itemHeight, setItemHeight] = useState(FALLBACK_ITEM_HEIGHT);
  const listRef = useRef<HTMLDivElement>(null);

  const presences = useMemo(
    () => gamePresenceIndex(openGames, liveGames),
    [liveGames, openGames],
  );

  // Collapsed categories are a persisted preference, not component state: this
  // list is remounted on every tab switch, so local state would forget the
  // choice constantly. The Java client persists it too.
  const hidden = useMemo(
    () => new Set(preferences.hiddenRosterCategories),
    [preferences.hiddenRosterCategories],
  );

  const toggleCategory = useCallback(
    (category: UserCategory) => {
      const current = useAppStore.getState().state.settings.chat;
      const next = current.hiddenRosterCategories.includes(category)
        ? current.hiddenRosterCategories.filter((entry) => entry !== category)
        : [...current.hiddenRosterCategories, category];
      ipc.send({
        kind: "Settings",
        command: { type: "setChat", payload: { preferences: { ...current, hiddenRosterCategories: next } } },
      });
    },
    [],
  );

  // Shared with the message list, which needs exactly the same index.
  const profilesByLogin = playersByNickname(social.players);
  const friendsSet = useMemo(
    () => new Set(social.friends.map((f) => f.toLowerCase())),
    [social.friends],
  );
  const foesSet = useMemo(
    () => new Set(social.foes.map((f) => f.toLowerCase())),
    [social.foes],
  );

  const groups = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    const matching = needle
      ? users.filter((u) => u.name.toLowerCase().includes(needle))
      : users;
    // Without the lobby we can't tell an account from an IRC-only nickname;
    // see `categoryOf`.
    const socialKnown = social.players.length > 0;
    const buckets = new Map<UserCategory, ChatUser[]>(
      USER_CATEGORY_ORDER.map((c) => [c, [] as ChatUser[]]),
    );
    for (const user of matching) {
      const lower = user.name.toLowerCase();
      let category: UserCategory;
      if (self && user.name === self) {
        category = "self";
      } else if (foesSet.has(lower)) {
        // Foes leave the normal list entirely: `USER_CATEGORY_ORDER` puts them
        // last, and this branch runs before the others so a foe stays there
        // even when they are a moderator or a known player.
        category = "foes";
      } else if (isModerator(user)) {
        category = "moderators";
      } else if (friendsSet.has(lower)) {
        category = "friends";
      } else if (!socialKnown || profilesByLogin.has(lower)) {
        category = "players";
      } else {
        category = "ircOnly";
      }
      buckets.get(category)?.push(user);
    }
    return buckets;
  }, [users, self, social, friendsSet, foesSet, profilesByLogin, filter]);

  // Flatten ordered categories and users into a unified virtualization list.
  const flatItems = useMemo<RosterFlatItem[]>(() => {
    const list: RosterFlatItem[] = [];
    for (const category of USER_CATEGORY_ORDER) {
      const bucket = groups.get(category) ?? [];
      if (bucket.length === 0) continue;
      const collapsed = hidden.has(category) && filter.trim() === "";
      list.push({
        type: "header",
        category,
        count: bucket.length,
        collapsed,
        key: `hdr-${category}`,
      });
      if (!collapsed) {
        for (const user of bucket) {
          list.push({
            type: "user",
            user,
            category,
            key: `usr-${user.name}`,
          });
        }
      }
    }
    return list;
  }, [groups, hidden, filter]);

  useLayoutEffect(() => {
    const el = listRef.current;
    if (!el) return;
    setViewportHeight(el.clientHeight || 600);
    setItemHeight(measureItemHeight(el));
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        if (entry.contentRect.height > 0) {
          setViewportHeight(entry.contentRect.height);
        }
      }
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const totalHeight = flatItems.length * itemHeight;
  const startIndex = Math.max(0, Math.floor(scrollTop / itemHeight) - OVERSCAN);
  const endIndex = Math.min(flatItems.length, Math.ceil((scrollTop + viewportHeight) / itemHeight) + OVERSCAN);
  const topPadding = startIndex * itemHeight;
  const visibleItems = flatItems.slice(startIndex, endIndex);

  return (
    <div className="chat-roster surface-panel" id="chat-roster">
      <div className="chat-roster-search">
        <label className="chat-roster-search-field">
          <Icon name="search" size={14} />
          <input
            type="search"
            value={filter}
            placeholder={t("chat.roster.filterPlaceholder", { count: users.length })}
            aria-label={t("chat.roster.filterAria", { count: users.length })}
            onChange={(e) => {
              setFilter(e.target.value);
              setScrollTop(0);
              if (listRef.current) listRef.current.scrollTop = 0;
            }}
          />
        </label>
      </div>

      <div
        className="chat-roster-list"
        ref={listRef}
        onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
      >
        {flatItems.length === 0 ? (
          users.length > 0 && (
            <p className="muted chat-empty">No user matches “{filter.trim()}”.</p>
          )
        ) : (
          <div className="chat-roster-virtual-track" style={{ height: totalHeight }}>
            <ul
              className="chat-roster-virtual-window"
              style={{ transform: `translateY(${topPadding}px)` }}
            >
              {visibleItems.map((item) => {
                if (item.type === "header") {
                  return (
                    <li key={item.key} className="chat-roster-group-header">
                      <h3>
                        <button
                          type="button"
                          className="chat-roster-heading"
                          aria-expanded={!item.collapsed}
                          title={
                            item.collapsed
                              ? `Show ${t(USER_CATEGORY_LABELS[item.category])}`
                              : `Hide ${t(USER_CATEGORY_LABELS[item.category])}`
                          }
                          onClick={() => toggleCategory(item.category)}
                        >
                          <Icon
                            name="chevronRight"
                            size={15}
                            className={
                              item.collapsed
                                ? "chat-roster-chevron"
                                : "chat-roster-chevron is-open"
                            }
                          />
                          <span className="chat-roster-heading-label">
                            {t(USER_CATEGORY_LABELS[item.category])}
                          </span>
                          <span className="chat-roster-count">{item.count}</span>
                        </button>
                      </h3>
                    </li>
                  );
                }
                return (
                  <RosterRow
                    key={item.key}
                    user={item.user}
                    profile={profilesByLogin.get(item.user.name.toLowerCase())}
                    social={social}
                    presence={presences.get(item.user.name.toLocaleLowerCase()) ?? null}
                    mapVault={mapVault}
                    preferences={preferences}
                    self={self}
                    onOpenConversation={onOpenConversation}
                    onContextMenu={onContextMenu}
                    menuOpen={menuTarget === item.user.name}
                  />
                );
              })}
            </ul>
          </div>
        )}
      </div>
    </div>
  );
});

const RosterRow = memo(function RosterRow({
  user,
  profile,
  social,
  presence,
  mapVault,
  preferences,
  self,
  onOpenConversation,
  onContextMenu,
  menuOpen,
}: {
  user: ChatUser;
  profile: PlayerProfile | undefined;
  social: SocialState;
  presence: GamePresence | null;
  mapVault: VaultMap[];
  preferences: ChatPreferences;
  self: string;
  onOpenConversation: (nick: string) => void;
  onContextMenu: (nick: string, event: React.MouseEvent) => void;
  menuOpen: boolean;
}) {
  const countryOf = useCountryLabel();
  const nameStyle = resolvedNickStyle(user.name, user, social, preferences, self);
  return (
    <li className="chat-roster-item">
      <div
        className={menuOpen ? "chat-roster-user is-menu-open" : "chat-roster-user"}
        onContextMenu={(e) => onContextMenu(user.name, e)}
      >
        <button
          type="button"
          className="chat-roster-identity"
          title={rosterRatingSummary(displayName(user.name, profile), profile)}
          onDoubleClick={() => onOpenConversation(user.name)}
        >
          <span className="chat-avatar">
            {profile?.avatarUrl ? (
              <img
                src={profile.avatarUrl}
                alt=""
                title={profile.avatarTooltip || undefined}
                width={40}
                height={20}
                loading="lazy"
                decoding="async"
                draggable={false}
              />
            ) : null}
          </span>
          {profile?.country ? (
            <img
              className="chat-flag"
              src={flagSrc(profile.country)}
              alt={countryOf(profile.country)}
              title={countryOf(profile.country)}
              width={16}
              height={16}
              decoding="async"
              draggable={false}
              onError={(e) => {
                e.currentTarget.style.visibility = "hidden";
              }}
            />
          ) : null}
          <span
            className={`chat-nick${nameStyle ? "" : " is-monochrome"}`}
            style={nameStyle}
          >
            {profile?.clan && <span className="chat-clan">[{profile.clan}]</span>}
            {user.name}
          </span>
        </button>
        {presence && <GameSummaryPopover presence={presence} social={social} vault={mapVault} />}
      </div>
    </li>
  );
});
