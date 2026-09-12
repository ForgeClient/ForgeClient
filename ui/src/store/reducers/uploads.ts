import type { UploadsEvent, UploadsState } from "../../ipc/bindings";

const EMPTY: UploadsState = { request: null, status: { type: "idle" }, preview: "" };

/** Twin of `UploadStatus::is_busy`, covering the stages where a publish is in flight. */
export function isUploadBusy(status: UploadsState["status"]): boolean {
  return status.type === "compressing" || status.type === "uploading" || status.type === "finishing";
}

export function reduceUploads(state: UploadsState, event: UploadsEvent): UploadsState {
  switch (event.type) {
    case "opened":
      // A fresh subject, so the previous map's picture goes with it.
      return { request: event.payload.request, status: { type: "idle" }, preview: "" };
    case "closed":
      // Closing does not cancel a publish already in flight; the bytes are
      // with the server. Keep the status so the next open cannot pretend
      // nothing is happening.
      return isUploadBusy(state.status) ? { ...state, request: null } : EMPTY;
    case "rankedChanged":
      return state.request === null
        ? state
        : { ...state, request: { ...state.request, ranked: event.payload.ranked } };
    case "progressed":
      return { ...state, status: event.payload.status };
    case "previewRead":
      return { ...state, preview: event.payload.dataUrl };
  }
}
