import { describe, expect, it } from "vitest";
import type { ChatMessage } from "../../ipc/bindings";
import { continuesPrevious } from "./messageFilters";

const message = (over: Partial<ChatMessage> = {}): ChatMessage => ({
  id: "1",
  sender: "Seraphim-Noob",
  content: "just a test ping",
  timestamp: "2026-09-12T10:00:00Z",
  kind: "message",
  ...over,
});

describe("grouping a run of messages from one person", () => {
  it("drops the name from the second line onward", () => {
    // The report: five lines, five identical names down the left of them.
    const first = message({ id: "1", timestamp: "2026-09-12T10:00:00Z" });
    const second = message({ id: "2", timestamp: "2026-09-12T10:00:20Z" });
    expect(continuesPrevious(second, first)).toBe(true);
  });

  it("starts a new block for a different person", () => {
    const first = message({ sender: "Nuggets" });
    expect(continuesPrevious(message(), first)).toBe(false);
  });

  it("starts a new block after a long pause", () => {
    const first = message({ timestamp: "2026-09-12T10:00:00Z" });
    const later = message({ timestamp: "2026-09-12T10:30:00Z" });
    expect(continuesPrevious(later, first)).toBe(false);
  });

  it("never groups the first line, an action, commentary, or a reply", () => {
    const first = message();
    expect(continuesPrevious(first, undefined)).toBe(false);
    expect(continuesPrevious(message({ kind: "action" }), first)).toBe(false);
    expect(continuesPrevious(message({ kind: "info" }), first)).toBe(false);
    // A reply carries a quote that needs a name under it to make sense of.
    expect(continuesPrevious(message({ replyTo: "abc" }), first)).toBe(false);
  });

  it("keeps the name when a timestamp cannot be read", () => {
    // The safe direction: a name too many is cosmetic, a missing one is not.
    const first = message({ timestamp: "not a time" });
    expect(continuesPrevious(message(), first)).toBe(false);
  });
});
