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
};

export const VOICE_MODE_RECORDING = "recording";
export const VOICE_MODE_TRANSCRIBING = "transcribing";
