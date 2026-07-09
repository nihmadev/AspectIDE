import { create } from "zustand";

import type { AspectModelUsage } from "./model-sync";

type AspectUsageStore = {
  map: Record<string, AspectModelUsage>;
  setMap: (map: Record<string, AspectModelUsage>) => void;
};

export const useAspectUsageStore = create<AspectUsageStore>((set) => ({
  map: {},
  setMap: (map) => set({ map }),
}));
