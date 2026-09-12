import { isValidElement } from "react";
import type { ReactNode } from "react";
import type { Game } from "../../ipc/bindings";
import { liveReplayLink } from "../../shared/replayLinks";
import { describe, expect, it } from "vitest";
import { parseChatGameLink, renderBody, renderFormattedText, stripHtmlTags } from "./chatFormat";

function linksIn(content: string): string[] {
  return renderBody(content, "").flatMap((node) =>
    isValidElement<{ href?: unknown }>(node)
      && node.type === "a"
      && typeof node.props.href === "string"
      ? [node.props.href]
      : []);
}

describe("chat link rendering", () => {
  it("linkifies ordinary HTTPS through the validated external-link path", () => {
    expect(linksIn("see https://example.org/match")).toEqual(["https://example.org/match"]);
  });

  it.each([
    "http://example.org/match",
    "https://user@example.org/match",
    "https://example.org:444/match",
  ])("leaves unsafe IRC URL text inert: %s", (url) => {
    expect(linksIn(`see ${url}`)).toEqual([]);
  });

  it("parses the Python client's open-game and live-replay URL grammar", () => {
    expect(parseChatGameLink(
      "fafgame://127.0.0.1:4567/42?map=dual_gap&mod=faf&mods=one%3Btwo&uid=123",
    )).toEqual({
      kind: "openGame",
      uid: 123,
      map: "dual_gap",
      mod: "faf",
      player: "42",
      mods: ["one", "two"],
    });
    expect(parseChatGameLink(
      "faflive://127.0.0.1/123/Foley.SCFAreplay?map=dual_gap&mod=faf",
    )).toMatchObject({ kind: "liveReplay", uid: 123, player: "Foley" });
  });

  it("accepts the live link emitted by the replay vault", () => {
    const game: Game = {
      id: 123,
      title: "Test",
      host: "Host",
      players: 2,
      maxPlayers: 4,
      map: "Seton's Clutch",
      modName: "faf",
      averageRating: 1500,
      ratingType: "global",
      passwordProtected: false,
      visibility: "public",
      gameType: "custom",
      launchedAt: null,
      hostedAt: null,
      ratingMin: null,
      ratingMax: null,
      enforceRatingRange: false,
      teams: {},
      simMods: {},
    };

    expect(parseChatGameLink(liveReplayLink(game, "Player One"))).toMatchObject({
      kind: "liveReplay",
      uid: 123,
      player: "Player One",
      map: "Seton's Clutch",
      mod: "faf",
    });
  });

  it.each([
    "fafgame://example.org/Foley?map=x&mod=faf&uid=1",
    "fafgame://127.0.0.1/Foley?map=x&mod=faf",
    "faflive://127.0.0.1/nope/Foley.SCFAreplay?map=x&mod=faf",
    "faflive://127.0.0.1/1/Foley.zip?map=x&mod=faf",
  ])("rejects malformed or non-local FAF game URLs: %s", (url) => {
    expect(parseChatGameLink(url)).toBeNull();
  });

  it("only makes a valid game URL interactive when an action is available", () => {
    const url = "fafgame://127.0.0.1/Foley?map=x&mod=faf&uid=1";
    const inert = renderBody(url, "");
    const active = renderBody(url, "", "", () => undefined);
    expect(inert.some((node) => isValidElement(node) && node.type === "button")).toBe(false);
    expect(active.some((node) => isValidElement(node) && node.type === "button")).toBe(true);
  });
});

describe("chat search highlighting", () => {
  it("highlights case-insensitive literal text without treating regex characters specially", () => {
    const nodes = renderBody("Find [FAF] and [faf]", "", "[faf]");
    const countMarks = (node: ReactNode): number => {
      if (Array.isArray(node)) return node.reduce<number>((total, child: ReactNode) => total + countMarks(child), 0);
      if (!isValidElement<{ children?: ReactNode }>(node)) return 0;
      return (node.type === "mark" ? 1 : 0) + countMarks(node.props.children);
    };
    expect(countMarks(nodes)).toBe(2);
  });
});

describe("who a coloured ping is shown to", () => {
  const roster = { names: new Set(["nuggets", "vindex"]), color: "#ff8c00" };

  /** Every `mark` in the tree, as `[className, colour, text]`. */
  const marks = (node: ReactNode): [string, string, string][] => {
    if (Array.isArray(node)) return node.flatMap((child: ReactNode) => marks(child));
    if (!isValidElement<{ children?: ReactNode; className?: string; style?: { color?: string } }>(node)) {
      return [];
    }
    const { children, className, style } = node.props;
    const inner = marks(children);
    // A mark wraps the matched token and nothing else, so its child is the
    // string; anything else is not a case this helper needs to describe.
    const text = typeof children === "string" ? children : "";
    return node.type === "mark"
      ? [[className ?? "", style?.color ?? "", text], ...inner]
      : inner;
  };

  it("colours every recognised name on a line we sent, so the sender sees it landed", () => {
    const nodes = renderBody("Nuggets and Vindex, look", "sheppy", "", undefined, roster, true);
    expect(marks(nodes).map(([, colour, text]) => [text, colour]))
      .toEqual([["Nuggets", "#ff8c00"], ["Vindex", "#ff8c00"]]);
  });

  it("colours only our own name on somebody else's line", () => {
    // The reported bug: a ping between two other people was painted on our
    // screen as though it concerned us.
    const nodes = renderBody("Nuggets and Vindex, look", "vindex", "", undefined, roster, false);
    expect(marks(nodes).map(([, , text]) => text)).toEqual(["Vindex"]);
  });

  it("colours nothing on somebody else's line that never names us", () => {
    const nodes = renderBody("Nuggets and Vindex, look", "sheppy", "", undefined, roster, false);
    expect(marks(nodes)).toEqual([]);
  });

  it("gives our own name the ping colour and no box", () => {
    const [[className, colour]] = marks(renderBody("hi Vindex", "vindex", "", undefined, roster, false));
    expect(className).toBe("chat-mention");
    expect(colour).toBe("#ff8c00");
  });
});

describe("server notice HTML tag parsing and stripping", () => {
  it("strips HTML anchor tags for native desktop notification popups", () => {
    const raw = 'Please download the client from <a href="https://www.faforever.com">https://www.faforever.com</a>';
    expect(stripHtmlTags(raw)).toBe("Please download the client from https://www.faforever.com");

    const custom = 'Visit the <a href="https://forum.faforever.com">FAF Forum</a>';
    expect(stripHtmlTags(custom)).toBe("Visit the FAF Forum (https://forum.faforever.com)");
  });

  it("parses server notice HTML anchor tags into clickable link elements", () => {
    const raw = 'Unofficial client notice: download from <a href="https://www.faforever.com">https://www.faforever.com</a>';
    const nodes = renderFormattedText(raw);
    const links = nodes.flatMap((node) =>
      isValidElement<{ href?: unknown }>(node) && node.type === "a" && typeof node.props.href === "string"
        ? [node.props.href]
        : [],
    );
    expect(links).toEqual(["https://www.faforever.com/"]);
  });
});
