/** Shared composer types/constants used across the decomposed composer sections. */
import type { RefObject } from "react";

export type AiComposerVoiceState = {
  canUseVoice: boolean;
  listening: boolean;
  toggleVoiceInput: () => void;
  /** Container for the mic button's live wave bars — the level meter writes
   *  --voice-level directly onto its children every animation frame (no re-renders). */
  voiceBarsRef: RefObject<HTMLSpanElement | null>;
  voiceError: string | null;
  voiceMode: string;
  voiceTitle: string;
};

export type AiComposerSelectOption = {
  label: string;
  value: string;
  group?: string;
  /** Trailing badge text (e.g. weekly "0/1000" usage for LuxIDE models). */
  badge?: string;
  /** Availability dot: "ok" (green) or "blocked" (red). */
  status?: "ok" | "blocked";
};

export const VOICE_MODE_RECORDING = "recording";
export const VOICE_MODE_TRANSCRIBING = "transcribing";
