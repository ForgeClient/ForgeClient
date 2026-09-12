import { FactionIcon } from "../../shared/FactionIcon";
import { useTranslation } from "../../i18n/useTranslation";

export const MATCHMAKER_FACTIONS = ["UEF", "Aeon", "Cybran", "Seraphim"] as const;

/** Matches `FACTION_NAMES` in `shared/factions.ts`, which the glyphs key off. */
const FACTION_IDS: Readonly<Record<string, number>> = {
  UEF: 1,
  Aeon: 2,
  Cybran: 3,
  Seraphim: 4,
};

interface Props {
  selected: string[];
  disabled: boolean;
  onChange: (factions: string[]) => void;
}

/**
 * Faction toggles, in the player card.
 *
 * These were four full-width cards on their own row: roughly a third of the
 * tab's height spent on four booleans. The Java client keeps them beside the
 * player identity for the same reason.
 *
 * They carry the faction's own colour when selected, which is what makes the
 * Java version readable at a glance: four grey chips differing only by a small
 * glyph make "which factions am I queuing as" a reading task rather than a
 * glance. `data-faction` picks the token; the colour never appears here.
 *
 * Glyph only, no written name, as in `team_matchmaking.fxml`, whose toggles
 * carry a bare `<Region styleClass="uef-icon"/>`. The row is cut to the width
 * of one game mode card and stays a single line, which four names would not
 * survive; the name is on the button as its accessible label and its tooltip.
 */
export function MatchmakerFactionPicker({ selected, disabled, onChange }: Props) {
  const { t } = useTranslation();
  const toggle = (faction: string) => {
    const next = selected.includes(faction)
      ? selected.filter((item) => item !== faction)
      : [...selected, faction];
    // The lobby does not accept an empty faction set. Match the Java client by
    // restoring the previous selection when the last faction is clicked off.
    if (next.length > 0) onChange(next);
  };

  return (
    <div className="faction-chips" role="group" aria-label={t("lobby.matchmaker.factions.aria")}>
      {MATCHMAKER_FACTIONS.map((faction) => {
        const active = selected.includes(faction);
        return (
          <button
            type="button"
            key={faction}
            data-faction={faction.toLocaleLowerCase()}
            disabled={disabled}
            aria-pressed={active}
            className={active ? "faction-chip is-active" : "faction-chip"}
            title={t(active ? "lobby.matchmaker.queuingAs" : "lobby.matchmaker.alsoQueueAs", {
              faction,
            })}
            onClick={() => toggle(faction)}
          >
            <FactionIcon faction={FACTION_IDS[faction]} size={22} />
            <span>{faction}</span>
          </button>
        );
      })}
    </div>
  );
}
