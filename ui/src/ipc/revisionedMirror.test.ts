import { describe, expect, it, vi } from "vitest";
import type { AppEvent, AppState } from "./bindings";
import { RevisionedMirror } from "./revisionedMirror";

const connecting: AppEvent = { kind: "Session", event: { type: "connecting" } };
const disconnected: AppEvent = { kind: "Session", event: { type: "disconnected" } };
const snapshotState = {} as AppState;

describe("RevisionedMirror", () => {
  it("does not replay an event already represented by the initial snapshot", () => {
    const hydrate = vi.fn();
    const apply = vi.fn();
    const mirror = new RevisionedMirror(hydrate, apply);

    mirror.receive({ kind: "event", revision: 4, event: connecting });
    mirror.replace({ revision: 4, state: snapshotState });

    expect(hydrate).toHaveBeenCalledOnce();
    expect(apply).not.toHaveBeenCalled();
  });

  it("replays later buffered events in revision order exactly once", () => {
    const apply = vi.fn();
    const mirror = new RevisionedMirror(vi.fn(), apply);

    mirror.receive({ kind: "event", revision: 7, event: disconnected });
    mirror.receive({ kind: "event", revision: 6, event: connecting });
    mirror.replace({ revision: 5, state: snapshotState });
    mirror.receive({ kind: "event", revision: 7, event: disconnected });

    expect(apply).toHaveBeenCalledTimes(2);
    expect(apply).toHaveBeenNthCalledWith(1, connecting);
    expect(apply).toHaveBeenNthCalledWith(2, disconnected);
  });

  it("replaces state at a lag-recovery boundary before applying later deltas", () => {
    const hydrate = vi.fn();
    const apply = vi.fn();
    const mirror = new RevisionedMirror(hydrate, apply);

    mirror.replace({ revision: 2, state: snapshotState });
    mirror.receive({ kind: "snapshot", revision: 9, state: snapshotState });
    mirror.receive({ kind: "event", revision: 10, event: disconnected });

    expect(hydrate).toHaveBeenCalledTimes(2);
    expect(apply).toHaveBeenCalledWith(disconnected);
  });

  it("does not roll state backward when an older snapshot completes late", () => {
    const hydrate = vi.fn();
    const apply = vi.fn();
    const mirror = new RevisionedMirror(hydrate, apply);

    mirror.replace({ revision: 5, state: snapshotState });
    mirror.replace({ revision: 4, state: {} as AppState });
    mirror.receive({ kind: "event", revision: 6, event: disconnected });

    expect(hydrate).toHaveBeenCalledOnce();
    expect(apply).toHaveBeenCalledWith(disconnected);
  });

  it("does not apply an event across a revision gap and recovers from a snapshot", async () => {
    const hydrate = vi.fn();
    const apply = vi.fn();
    const resnapshot = vi.fn().mockResolvedValue({ revision: 4, state: snapshotState });
    const mirror = new RevisionedMirror(hydrate, apply, resnapshot);

    mirror.replace({ revision: 2, state: snapshotState });
    mirror.receive({ kind: "event", revision: 5, event: disconnected });

    expect(apply).not.toHaveBeenCalled();
    expect(resnapshot).toHaveBeenCalledOnce();
    await vi.waitFor(() => expect(apply).toHaveBeenCalledWith(disconnected));
    expect(hydrate).toHaveBeenCalledTimes(2);
  });

  it("coalesces several gaps into one recovery request", async () => {
    let finishRecovery!: (snapshot: { revision: number; state: AppState }) => void;
    const resnapshot = vi.fn(() => new Promise<{ revision: number; state: AppState }>((resolve) => {
      finishRecovery = resolve;
    }));
    const mirror = new RevisionedMirror(vi.fn(), vi.fn(), resnapshot);

    mirror.replace({ revision: 1, state: snapshotState });
    mirror.receive({ kind: "event", revision: 3, event: connecting });
    mirror.receive({ kind: "event", revision: 4, event: disconnected });
    expect(resnapshot).toHaveBeenCalledOnce();

    finishRecovery({ revision: 4, state: snapshotState });
    await vi.waitFor(() => expect(resnapshot).toHaveBeenCalledOnce());
  });

  it("does not ask for a second snapshot straight after the first", async () => {
    // A gap means the client is already behind, and a snapshot is megabytes.
    // Two gaps in quick succession used to mean two of them, which is the
    // lag cascade the cooldown exists to break.
    const resnapshot = vi.fn().mockResolvedValue({ revision: 2, state: snapshotState });
    const mirror = new RevisionedMirror(vi.fn(), vi.fn(), resnapshot);

    mirror.replace({ revision: 1, state: snapshotState });
    mirror.receive({ kind: "event", revision: 3, event: connecting });
    await vi.waitFor(() => expect(resnapshot).toHaveBeenCalledOnce());

    mirror.receive({ kind: "event", revision: 9, event: disconnected });
    // Still one: the second gap is remembered, not refetched immediately.
    expect(resnapshot).toHaveBeenCalledOnce();
  });

  it("reports recovery failures without applying the out-of-order event", async () => {
    const apply = vi.fn();
    const onError = vi.fn();
    const mirror = new RevisionedMirror(
      vi.fn(),
      apply,
      () => Promise.reject(new Error("offline")),
      onError,
    );
    mirror.replace({ revision: 1, state: snapshotState });
    mirror.receive({ kind: "event", revision: 3, event: disconnected });

    await vi.waitFor(() => expect(onError).toHaveBeenCalledOnce());
    expect(apply).not.toHaveBeenCalled();
  });
});
