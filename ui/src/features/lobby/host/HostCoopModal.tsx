// Hosting a co-op mission.
//
// The custom-game dialog's first column asks which featured mod to run, and in
// co-op that question has one answer: the mod is locked to `coop`, so all four
// rows were rendered with three of them disabled. That column is the campaign
// list here, the map list narrows to the missions of the chosen campaign, and
// the map generator is gone: a generated map is not a campaign mission.

import { memo, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../../../design-system/Button";
import { Icon } from "../../../design-system/Icon";
import { Modal } from "../../../design-system/Modal";
import { ipc } from "../../../ipc/client";
import type { CoopMission } from "../../../ipc/bindings";
import { useTranslation } from "../../../i18n/useTranslation";
import { useAppStore } from "../../../store/store";
import { focusListboxOption, nextListboxIndex } from "../../../shared/listboxNavigation";
import { loadLocalMapPreviews } from "../../../shared/useLocalMapPreview";
import { scenarioBadge, sortCoopScenarios } from "../coopScenarios";
import { CoopMissionArt } from "./CoopMissionArt";
import { HostModsColumn } from "./HostModsColumn";
import { HostTopConfig } from "./HostTopConfig";
import { useHostLobbySettings } from "./hostLobbySettings";

/** The featured mod a co-op lobby always runs. */
const COOP_FEATURED_MOD = "coop";

/** Stands for "the missions no campaign claims"; see `campaigns` below. */
const NO_CAMPAIGN = -1;

interface Props {
  onClose: () => void;
  /** Which mission to open on, normally the one the leaderboard is showing. */
  initialMissionId?: number | null;
  /** A title another tab prepared, which then wins over the mission's own. */
  initialTitle?: string;
}

/** Memoised for the same reason [`HostGameModal`] is: the Play tab it hangs
 *  off re-renders on every game list the lobby sends, and nothing in here
 *  reads one. */
export const HostCoopModal = memo(function HostCoopModal({ onClose, initialMissionId, initialTitle }: Props) {
  const { t } = useTranslation();
  const coop = useAppStore((state) => state.state.coop);
  const vault = useAppStore((state) => state.state.maps.vault);
  const remembered = useAppStore((state) => state.state.settings.browsing.hostCoop);
  const settings = useHostLobbySettings(initialTitle, "hostCoop");

  // Whether the host has written their own title. Until they do, it follows
  // the selected mission, which is what makes clicking through the list
  // useful: the lobby name is right without anyone typing it.
  // A remembered title counts as written too: it was typed in an earlier
  // visit, and letting the mission's name overwrite it would undo exactly the
  // thing this dialog now remembers.
  const [titleTouched, setTitleTouched] = useState(
    initialTitle !== undefined || remembered.title !== "",
  );
  const [missionSearch, setMissionSearch] = useState("");
  const [campaignId, setCampaignId] = useState<number | null>(null);
  // Where the mission comes from, in order: an explicit "host this one" from
  // the leaderboard, then the mission this dialog was last on, then whatever
  // the campaign lists first. The middle one is what survives leaving the tab:
  // the play view is unmounted on every tab switch, taking this state with it.
  const [missionId, setMissionId] = useState<number | null>(() => {
    if (initialMissionId != null) return initialMissionId;
    const folder = remembered.map.toLocaleLowerCase();
    if (!folder) return null;
    const previous = useAppStore
      .getState()
      .state.coop.missions.find(
        (mission) => mission.mapFolderName.toLocaleLowerCase() === folder,
      );
    return previous?.id ?? null;
  });
  const missionListRef = useRef<HTMLDivElement>(null);
  const campaignListRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (useAppStore.getState().state.coop.catalogStatus.type === "idle") {
      ipc.send({ kind: "Coop", command: { type: "loadCatalog" } });
    }
    ipc.send({ kind: "Maps", command: { type: "loadInstalled" } });
    if (useAppStore.getState().state.maps.vaultStatus.type === "idle") {
      ipc.send({ kind: "Maps", command: { type: "loadVault" } });
    }
    ipc.send({ kind: "Mods", command: { type: "loadInstalled" } });
  }, []);

  // A mission's campaign is only expressed one way in the API (a campaign
  // lists its maps), and the inversion leaves missions nothing claims. They
  // used to be unreachable: the old dropdown filtered strictly by campaign, so
  // a mission with no owner appeared under none of them. The bucket is real,
  // but it only earns a row when something is actually in it.
  const orphanCount = useMemo(
    () => coop.missions.filter((mission) => mission.scenarioId === null).length,
    [coop.missions],
  );

  const campaigns = useMemo(() => {
    const sorted = sortCoopScenarios(coop.scenarios);
    return orphanCount > 0
      ? [
          ...sorted,
          {
            id: NO_CAMPAIGN,
            name: t("lobby.coop.withoutCampaign"),
            faction: "custom" as const,
            order: Number.MAX_SAFE_INTEGER,
            description: "",
            category: "custom" as const,
          },
        ]
      : sorted;
  }, [coop.scenarios, orphanCount, t]);

  // The campaign of the mission we opened on, so "host this one" lands on it.
  const initialCampaignId = useMemo(() => {
    const mission = coop.missions.find((entry) => entry.id === initialMissionId);
    if (!mission) return null;
    return mission.scenarioId ?? NO_CAMPAIGN;
  }, [coop.missions, initialMissionId]);

  const activeCampaignId = campaignId ?? initialCampaignId ?? campaigns[0]?.id ?? null;
  const activeCampaign = campaigns.find((entry) => entry.id === activeCampaignId);

  const missionsInCampaign = useMemo(() => {
    const search = missionSearch.trim().toLocaleLowerCase();
    return coop.missions
      .filter((mission) =>
        activeCampaignId === NO_CAMPAIGN
          ? mission.scenarioId === null
          : mission.scenarioId === activeCampaignId,
      )
      .filter(
        (mission) =>
          !search ||
          mission.name.toLocaleLowerCase().includes(search) ||
          mission.mapFolderName.toLocaleLowerCase().includes(search),
      )
      .sort((a, b) => a.name.localeCompare(b.name));
  }, [activeCampaignId, coop.missions, missionSearch]);

  const selected: CoopMission | undefined =
    missionsInCampaign.find((mission) => mission.id === missionId) ?? missionsInCampaign[0];

  // One batch per campaign rather than one read per click: the art lives in the
  // installed map folders, and reading twenty of them once is cheaper than
  // waiting for a disk read every time the selection moves.
  useEffect(() => {
    if (missionsInCampaign.length === 0) return;
    loadLocalMapPreviews(missionsInCampaign.map((mission) => mission.mapFolderName));
  }, [missionsInCampaign]);

  useEffect(() => {
    if (!selected || titleTouched) return;
    settings.setTitle(t("lobby.coop.defaultTitle", { mission: selected.name }));
    // `settings` is rebuilt every render; the mission is what this reacts to.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected?.id, titleTouched]);

  // Remember the form on the way out rather than on every keystroke: one
  // settings write per dialog, and being unmounted by a tab switch is exactly
  // the case this is for. A ref because the cleanup runs once, with whatever
  // the form held at that moment.
  const draft = useRef(() => remembered);
  draft.current = () => settings.remembered(COOP_FEATURED_MOD, selected?.mapFolderName ?? "");
  useEffect(
    () => () => {
      const browsing = useAppStore.getState().state.settings.browsing;
      const hostCoop = draft.current();
      // Nothing changed: a dialog opened and closed again should not cost a
      // settings write.
      if (JSON.stringify(browsing.hostCoop) === JSON.stringify(hostCoop)) return;
      ipc.send({
        kind: "Settings",
        command: { type: "setBrowsing", payload: { preferences: { ...browsing, hostCoop } } },
      });
    },
    [],
  );

  const scenario = coop.scenarios.find((entry) => entry.id === selected?.scenarioId);

  const formError =
    settings.titleError ||
    settings.passwordError ||
    settings.ratingError ||
    (!selected ? t("lobby.host.error.selectMission") : "");

  const chooseRandom = () => {
    if (missionsInCampaign.length === 0) return;
    const index = Math.floor(Math.random() * missionsInCampaign.length);
    setMissionId(missionsInCampaign[index].id);
  };

  /// And the campaign column beside it, which narrows the mission list.
  const onCampaignListKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const current = campaigns.findIndex((campaign) => campaign.id === activeCampaignId);
    const next = nextListboxIndex(event.key, current, campaigns.length);
    if (next === null) return;
    event.preventDefault();
    setCampaignId(campaigns[next].id);
    setMissionId(null);
    setMissionSearch("");
    focusListboxOption(campaignListRef.current, next);
  };

  /// The mission list's half of the same keyboard navigation the map list has:
  /// the mission art and its briefing follow the selection, not focus.
  const onMissionListKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const current = missionsInCampaign.findIndex((mission) => mission.id === selected?.id);
    const next = nextListboxIndex(event.key, current, missionsInCampaign.length);
    if (next === null) return;
    event.preventDefault();
    setMissionId(missionsInCampaign[next].id);
    focusListboxOption(missionListRef.current, next);
  };

  const host = () => {
    if (formError || !selected) return;
    ipc.send({
      kind: "Lobby",
      command: {
        type: "host",
        payload: {
          config: {
            ...settings.hostConfig(),
            modName: COOP_FEATURED_MOD,
            map: selected.mapFolderName,
          },
        },
      },
    });
    // Leave the leaderboard on the mission that was just hosted: the records
    // for it are what the host wants to see behind the lobby.
    ipc.send({
      kind: "Coop",
      command: { type: "selectMission", payload: { missionId: selected.id } },
    });
    onClose();
  };

  return (
    <Modal className="host-game-modal host-coop-modal" onClose={onClose}>
      <div className="play-dialog-head">
        <div>
          <h2>{t("lobby.host.titleCoop")}</h2>
          <p>{t("lobby.coop.hostSubtitle")}</p>
        </div>
      </div>

      <HostTopConfig
        settings={{
          ...settings,
          setTitle: (value) => {
            setTitleTouched(true);
            settings.setTitle(value);
          },
        }}
      />

      <div className="host-game-grid host-coop-grid">
        {/* Column 1: campaigns, where the custom dialog asks for a featured mod. */}
        <section className="host-column host-column-gametype surface-panel">
          <div className="host-column-header">
            <h3>{t("lobby.coop.campaigns")}</h3>
          </div>
          <div
            ref={campaignListRef}
            className="host-column-body host-gametype-list"
            role="listbox"
            aria-label={t("lobby.coop.campaigns")}
            onKeyDown={onCampaignListKeyDown}
          >
            {campaigns.length === 0 ? (
              <p className="play-empty">{t("lobby.coop.loadingMissions")}</p>
            ) : (
              campaigns.map((campaign) => {
                const active = campaign.id === activeCampaignId;
                return (
                  <button
                    key={campaign.id}
                    type="button"
                    role="option"
                    aria-selected={active}
                    className={`host-gametype-row${active ? " active" : ""}`}
                    onClick={() => {
                      setCampaignId(campaign.id);
                      setMissionId(null);
                      setMissionSearch("");
                    }}
                  >
                    <div className="host-gametype-title-row">
                      <span className="host-gametype-name">{campaign.name}</span>
                      <span className="host-coop-faction-badge" data-faction={scenarioBadge(campaign)}>
                        {t(`lobby.coop.badge.${scenarioBadge(campaign)}`)}
                      </span>
                    </div>
                  </button>
                );
              })
            )}
          </div>
        </section>

        {/* Column 2: the missions of that campaign, not the whole map vault. */}
        <section className="host-column host-column-maps surface-panel">
          <div className="host-column-header">
            <h3>{t("lobby.coop.mission")}</h3>
            <span className="host-count-badge">
              {t("lobby.coop.missionCount", { count: missionsInCampaign.length })}
            </span>
          </div>

          <div className="search-field host-column-search">
            <Icon name="search" size={13} />
            <input
              value={missionSearch}
              onChange={(event) => setMissionSearch(event.target.value)}
              placeholder={t("lobby.coop.searchMissionsPlaceholder")}
              aria-label={t("lobby.coop.searchMissionsAria")}
            />
          </div>

          <div
            ref={missionListRef}
            className="host-column-body host-map-list"
            role="listbox"
            aria-label={t("lobby.coop.missionListAria")}
            onKeyDown={onMissionListKeyDown}
          >
            {missionsInCampaign.length === 0 ? (
              <p className="play-empty">{t("lobby.coop.noMissions")}</p>
            ) : (
              missionsInCampaign.map((mission) => (
                <button
                  key={mission.id}
                  type="button"
                  role="option"
                  aria-selected={selected?.id === mission.id}
                  className={`host-map-row${selected?.id === mission.id ? " active" : ""}`}
                  onClick={() => setMissionId(mission.id)}
                >
                  <span className="host-map-name" title={mission.name}>
                    {mission.name}
                  </span>
                  <span className="host-map-meta">{mission.mapFolderName}</span>
                </button>
              ))
            )}
          </div>

          <div className="host-column-footer host-map-actions">
            <Button
              className="host-col-action-btn"
              disabled={missionsInCampaign.length === 0}
              onClick={chooseRandom}
            >
              <Icon name="refresh" size={14} />
              {t("lobby.coop.randomMission")}
            </Button>
          </div>
        </section>

        {/* Column 3: the mission's art and briefing. */}
        <section className="host-column host-column-preview surface-panel">
          <div className="host-column-header">
            <h3>{t("lobby.coop.selectedMission")}</h3>
          </div>

          <div className="host-column-body host-preview-body">
            <div className="host-preview-thumb-wrap">
              {selected ? (
                <CoopMissionArt
                  mission={selected}
                  scenario={scenario}
                  vault={vault}
                  className="host-preview-img"
                />
              ) : (
                <div className="host-preview-placeholder">
                  <Icon name="maps" size={32} />
                </div>
              )}
            </div>

            {/* The name under the art rather than over it, matching the custom
                host dialog: the overlay dimmed the corner of every preview to
                repeat what the row below already says. */}
            <div className="host-preview-name">
              <span title={selected?.name}>
                {selected?.name ?? t("lobby.coop.selectMission")}
              </span>
            </div>

            {selected && (
              <div className="host-map-info-section">
                <dl className="host-map-facts">
                  <dt>{t("lobby.coop.campaign")}</dt>
                  <dd>{activeCampaign?.name ?? t("lobby.coop.withoutCampaign")}</dd>
                  <dt>{t("lobby.coop.mapFolder")}</dt>
                  <dd>{selected.mapFolderName}</dd>
                </dl>
                {selected.description && (
                  <p className="host-map-description">{selected.description}</p>
                )}
              </div>
            )}
          </div>
        </section>

        <HostModsColumn />
      </div>

      <div className="play-dialog-actions">
        {formError && <span className="host-form-global-error">{formError}</span>}
        <Button onClick={onClose}>{t("lobby.host.cancel")}</Button>
        <Button variant="primary" disabled={Boolean(formError)} onClick={host}>
          {t("lobby.host.submit")}
        </Button>
      </div>
    </Modal>
  );
});
