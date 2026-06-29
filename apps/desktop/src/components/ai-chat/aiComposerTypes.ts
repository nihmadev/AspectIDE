/** Shared composer types/constants used across the decomposed composer sections. */

export type AiComposerVoiceState = {
  canUseVoice: boolean;
  listening: boolean;
  toggleVoiceInput: () => void;
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
