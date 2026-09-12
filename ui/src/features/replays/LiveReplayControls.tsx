import { Button } from "../../design-system/Button";
import { Icon } from "../../design-system/Icon";
import { prettyFeaturedMod, prettyGameType, type LiveFilters } from "./liveReplayModel";
import { useTranslation } from "../../i18n/useTranslation";

interface Props {
  filters: LiveFilters;
  filtersOpen: boolean;
  activeFilterCount: number;
  gameTypes: string[];
  featuredMods: string[];
  activePlayerOptions: number[];
  maxPlayerOptions: number[];
  onFilter: <K extends keyof LiveFilters>(key: K, value: LiveFilters[K]) => void;
  onToggleFilters: () => void;
  onClear: () => void;
}

export function LiveReplayControls(props: Props) {
  const { t } = useTranslation();
  const { filters, onFilter } = props;
  return (
    <>
      <div className="live-replay-toolbar">
        <label className="search-field live-replay-search">
          <Icon name="search" size={15} />
          <input
            value={filters.search}
            onChange={(event) => onFilter("search", event.target.value)}
            placeholder={t("replays.live.searchPlaceholder")}
            aria-label={t("replays.live.searchAria")}
          />
        </label>
        <label className="toolbar-check">
          <input
            type="checkbox"
            checked={filters.hideModded}
            onChange={(event) => onFilter("hideModded", event.target.checked)}
          />
          {t("replays.live.hideModded")}
        </label>
        <label className="toolbar-check">
          <input
            type="checkbox"
            checked={filters.hideSinglePlayer}
            onChange={(event) => onFilter("hideSinglePlayer", event.target.checked)}
          />
          {t("replays.live.hideSinglePlayer")}
        </label>
        <label className="toolbar-check">
          <input
            type="checkbox"
            checked={filters.friendsOnly}
            onChange={(event) => onFilter("friendsOnly", event.target.checked)}
          />
          {t("replays.live.friendsOnly")}
        </label>
        <Button
          className={props.filtersOpen ? "live-filter-button active" : "live-filter-button"}
          aria-expanded={props.filtersOpen}
          onClick={props.onToggleFilters}
        >
          <Icon name="filter" size={15} />
          {props.activeFilterCount > 0
            ? t("replays.live.filtersCount", { count: props.activeFilterCount })
            : t("replays.live.filters")}
        </Button>
        {props.activeFilterCount > 0 && <Button onClick={props.onClear}>{t("replays.live.clear")}</Button>}
        <span className="live-replay-stream-status">
          <i aria-hidden="true" /> {t("replays.live.updates")}
        </span>
      </div>

      {props.filtersOpen && (
        <div className="live-replay-filters surface-panel">
          <label>
            <span>{t("replays.live.gameType")}</span>
            <select value={filters.gameType} onChange={(event) => onFilter("gameType", event.target.value)}>
              <option value="">{t("replays.live.anyType")}</option>
              {props.gameTypes.map((type) => <option key={type} value={type}>{prettyGameType(type)}</option>)}
            </select>
          </label>
          <label>
            <span>{t("replays.live.featuredMod")}</span>
            <select value={filters.featuredMod} onChange={(event) => onFilter("featuredMod", event.target.value)}>
              <option value="">{t("replays.live.anyMod")}</option>
              {props.featuredMods.map((mod) => <option key={mod} value={mod}>{prettyFeaturedMod(mod)}</option>)}
            </select>
          </label>
          <label>
            <span>{t("replays.live.activePlayers")}</span>
            <select value={filters.activePlayers} onChange={(event) => onFilter("activePlayers", event.target.value)}>
              <option value="">{t("replays.live.anyCount")}</option>
              {props.activePlayerOptions.map((count) => <option key={count} value={count}>{count}</option>)}
            </select>
          </label>
          <label>
            <span>{t("replays.live.gameSize")}</span>
            <select value={filters.maxPlayers} onChange={(event) => onFilter("maxPlayers", event.target.value)}>
              <option value="">{t("replays.live.anySize")}</option>
              {props.maxPlayerOptions.map((count) => (
                <option key={count} value={count}>{t("replays.live.slots", { count })}</option>
              ))}
            </select>
          </label>
        </div>
      )}
    </>
  );
}
