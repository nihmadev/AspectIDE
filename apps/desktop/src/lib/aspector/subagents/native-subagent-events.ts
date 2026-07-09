// App-lifetime bridge for subagent progress events.
//
// Subagent progress (`kind: "subagentProgress"`) must OUTLIVE the turn that
// spawned the subagent: a background Task (Task {background:true}) keeps
// running after its parent turn settles, and the per-turn listener in
// aiNativeTurn.ts unsubscribes at settle — without this global listener the
// detached subagent's rail row would freeze "running" forever, its final
// done/error stage lost. One session-independent listener routes every
// progress event by callId; the bridge ignores unknown or settled runs.

import { subscribeAiTurn } from "./../../tauri/commands";
import { bridgeNativeSubagentProgress } from "./native-orchestration-bridge";

let installed = false;

/** Install the global listener once (idempotent; retried if subscribe fails). */
export function ensureNativeSubagentProgressBridge() {
  if (installed) return;
  installed = true;
  subscribeAiTurn((event) => {
    if (event.kind === "subagentProgress") {
      bridgeNativeSubagentProgress(event.callId, event.stage, event.content, event.tool);
    }
  }).catch(() => {
    // Subscription failed (e.g. non-Tauri test runtime) — allow a retry later.
    installed = false;
  });
}
