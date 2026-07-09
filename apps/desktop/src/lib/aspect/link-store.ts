import { create } from "zustand";

export type AspectLinkPhase = "starting" | "waiting" | "linked" | "error";

type AspectLinkStore = {
  open: boolean;
  phase: AspectLinkPhase;
  code: string;
  deepLink: string;
  error: string;
  show: (patch: Partial<Pick<AspectLinkStore, "phase" | "code" | "deepLink" | "error">>) => void;
  hide: () => void;
};

export const useAspectLinkStore = create<AspectLinkStore>((set) => ({
  open: false,
  phase: "starting",
  code: "",
  deepLink: "",
  error: "",
  show: (patch) => set((state) => ({ ...state, open: true, ...patch })),
  hide: () => set({ open: false, phase: "starting", code: "", deepLink: "", error: "" }),
}));
