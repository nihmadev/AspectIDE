import { create } from "zustand";

/** Phases of the Telegram device-link flow shown in the modal. */
export type LuxideLinkPhase = "starting" | "waiting" | "linked" | "error";

type LuxideLinkStore = {
  open: boolean;
  phase: LuxideLinkPhase;
  code: string;
  deepLink: string;
  error: string;
  /** Open the modal (or update it) with a partial patch. */
  show: (patch: Partial<Pick<LuxideLinkStore, "phase" | "code" | "deepLink" | "error">>) => void;
  /** Close the modal and reset. Also signals the in-flight link flow to stop. */
  hide: () => void;
};

/**
 * Dedicated store for the LuxIDE Telegram-link modal. Kept separate from the main
 * app store so the link flow (luxideEnroll.ts) and the modal component share state
 * without threading it through the large LuxState.
 */
export const useLuxideLinkStore = create<LuxideLinkStore>((set) => ({
  open: false,
  phase: "starting",
  code: "",
  deepLink: "",
  error: "",
  show: (patch) => set((state) => ({ ...state, open: true, ...patch })),
  hide: () => set({ open: false, phase: "starting", code: "", deepLink: "", error: "" }),
}));
