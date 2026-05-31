import { useCallback, useEffect, useRef, useState } from "react";
import type { AiPreferences, AiVoiceInputLanguage } from "./aiPreferences";
import { useTranslation, type TranslateFn } from "./i18n/useTranslation";
import { luxCommands, type VoiceInputProviderStatus } from "./tauri";

type SpeechRecognitionResultLike = {
  isFinal: boolean;
  0: { transcript: string };
};

type SpeechRecognitionEventLike = {
  resultIndex: number;
  results: SpeechRecognitionResultLike[];
};

type SpeechRecognitionLike = {
  continuous: boolean;
  interimResults: boolean;
  lang: string;
  onend: (() => void) | null;
  onerror: ((event: SpeechRecognitionErrorEventLike) => void) | null;
  onresult: ((event: SpeechRecognitionEventLike) => void) | null;
  start: () => void;
  stop: () => void;
};

type SpeechRecognitionErrorEventLike = {
  error?: string;
  message?: string;
};

type SpeechRecognitionConstructor = new () => SpeechRecognitionLike;

type WindowWithSpeech = Window & {
  SpeechRecognition?: SpeechRecognitionConstructor;
  webkitSpeechRecognition?: SpeechRecognitionConstructor;
  MediaRecorder?: typeof MediaRecorder & { isTypeSupported?: (mimeType: string) => boolean };
};

type VoiceInputMode = "idle" | "recording" | "transcribing";

type UseVoiceInputOptions = {
  message: string;
  preferences: AiPreferences;
  updateMessage: (nextMessage: string) => void;
};

export function useVoiceInput({ message, preferences, updateMessage }: UseVoiceInputOptions) {
  const { t } = useTranslation();
  const [listening, setListening] = useState(false);
  const [voiceAvailable, setVoiceAvailable] = useState(false);
  const [voiceProvider, setVoiceProvider] = useState<"native" | "webkit" | null>(null);
  const [voiceError, setVoiceError] = useState<string | null>(null);
  const [voiceStatus, setVoiceStatus] = useState<VoiceInputProviderStatus | null>(null);
  const [voiceMode, setVoiceMode] = useState<VoiceInputMode>("idle");
  const recognitionRef = useRef<SpeechRecognitionLike | null>(null);
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const mediaStreamRef = useRef<MediaStream | null>(null);
  const recordedChunksRef = useRef<Blob[]>([]);
  const shouldTranscribeRecordingRef = useRef(true);
  const latestMessageRef = useRef(message);
  const voiceBaseMessageRef = useRef("");
  const voiceFinalTranscriptRef = useRef("");
  const voiceInterimTranscriptRef = useRef("");

  const voiceSelectedProvider = preferences.voiceInputProvider;
  const canUseVoice = preferences.voiceInputEnabled
    && (voiceSelectedProvider === "native-webview" ? voiceAvailable : Boolean(voiceStatus?.available));
  const voiceLabel = voiceStatusLabel(t, {
    enabled: preferences.voiceInputEnabled,
    error: voiceError,
    listening,
    mode: voiceMode,
    provider: voiceProvider,
    selectedProvider: voiceSelectedProvider,
    status: voiceStatus,
  });
  const voiceTitle = voiceButtonTitle(t, {
    enabled: preferences.voiceInputEnabled,
    error: voiceError,
    mode: voiceMode,
    provider: voiceProvider,
    selectedProvider: voiceSelectedProvider,
    status: voiceStatus,
    voiceAvailable,
  });

  useEffect(() => {
    latestMessageRef.current = message;
  }, [message]);

  const appendTranscript = useCallback((transcript: string) => {
    const text = transcript.trim();
    if (!text) return;
    const current = latestMessageRef.current;
    updateMessage(`${current}${current.trim() ? " " : ""}${text}`);
  }, [updateMessage]);

  const resetLiveTranscript = useCallback((baseMessage: string) => {
    voiceBaseMessageRef.current = baseMessage;
    voiceFinalTranscriptRef.current = "";
    voiceInterimTranscriptRef.current = "";
  }, []);

  const renderLiveTranscript = useCallback(() => {
    const base = voiceBaseMessageRef.current.trimEnd();
    const spoken = [voiceFinalTranscriptRef.current, voiceInterimTranscriptRef.current]
      .map((part) => part.trim())
      .filter(Boolean)
      .join(" ");
    updateMessage(spoken ? `${base}${base ? " " : ""}${spoken}` : base);
  }, [updateMessage]);

  useEffect(() => {
    if (!preferences.voiceInputEnabled && listening) {
      stopNativeRecognition();
      setListening(false);
    }
    if (!preferences.voiceInputEnabled && voiceMode !== "idle") {
      stopLocalVoiceRecording(false);
    }
  }, [preferences.voiceInputEnabled, listening, voiceMode]);

  useEffect(() => {
    const speechWindow = window as WindowWithSpeech;
    const provider = speechWindow.SpeechRecognition ? "native" : speechWindow.webkitSpeechRecognition ? "webkit" : null;
    setVoiceProvider(provider);
    setVoiceAvailable(Boolean(provider));
    return () => {
      try {
        recognitionRef.current?.stop();
      } catch {
        undefined;
      }
      recognitionRef.current = null;
      stopLocalVoiceRecording(false);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    void luxCommands.voiceInputStatus(
      preferences.voiceInputProvider,
      preferences.localSttCommand || null,
      preferences.localSttModelPath || null,
    ).then((status) => {
      if (cancelled) return;
      setVoiceStatus(status);
      if (preferences.voiceInputProvider === "local") setVoiceAvailable(status.available);
    }).catch((error) => {
      if (cancelled) return;
      setVoiceStatus({
        provider: preferences.voiceInputProvider,
        available: false,
        detail: readErrorMessage(t, error),
        command: preferences.localSttCommand || null,
        modelPath: preferences.localSttModelPath || null,
      });
      if (preferences.voiceInputProvider === "local") setVoiceAvailable(false);
    });
    return () => {
      cancelled = true;
    };
  }, [preferences.localSttCommand, preferences.localSttModelPath, preferences.voiceInputProvider]);

  const toggleVoiceInput = () => {
    if (voiceSelectedProvider === "local") {
      void toggleLocalVoiceInput();
      return;
    }
    toggleNativeVoiceInput();
  };

  const toggleNativeVoiceInput = () => {
    if (listening) {
      stopNativeRecognition();
      setListening(false);
      return;
    }

    if (!preferences.voiceInputEnabled) {
      setVoiceError(t("voice.error.disabled"));
      return;
    }

    const speechWindow = window as WindowWithSpeech;
    const Recognition = speechWindow.SpeechRecognition ?? speechWindow.webkitSpeechRecognition;
    if (!Recognition) {
      setVoiceAvailable(false);
      setVoiceError(t("voice.error.nativeUnavailable"));
      return;
    }

    setVoiceError(null);
    resetLiveTranscript(latestMessageRef.current);
    try {
      const recognition = new Recognition();
      recognition.continuous = true;
      recognition.interimResults = true;
      recognition.lang = resolveVoiceLanguage(preferences.voiceInputLanguage);
      recognition.onresult = (event) => {
        try {
          const finalParts: string[] = [];
          const interimParts: string[] = [];
          for (let index = 0; index < event.results.length; index += 1) {
            const result = event.results[index];
            const transcript = result?.[0]?.transcript ?? "";
            if (!transcript.trim()) continue;
            if (result.isFinal) finalParts.push(transcript);
            else interimParts.push(transcript);
          }
          voiceFinalTranscriptRef.current = finalParts.join(" ").trim();
          voiceInterimTranscriptRef.current = interimParts.join(" ").trim();
          renderLiveTranscript();
        } catch (error) {
          setVoiceError(readErrorMessage(t, error));
          setListening(false);
        }
      };
      recognition.onerror = (event) => {
        setVoiceError(formatVoiceError(t, event));
        recognitionRef.current = null;
        setListening(false);
      };
      recognition.onend = () => {
        voiceInterimTranscriptRef.current = "";
        renderLiveTranscript();
        recognitionRef.current = null;
        setListening(false);
      };
      recognitionRef.current = recognition;
      recognition.start();
      setListening(true);
    } catch (error) {
      recognitionRef.current = null;
      setListening(false);
      setVoiceError(readErrorMessage(t, error));
    }
  };

  function stopNativeRecognition() {
    const recognition = recognitionRef.current;
    recognitionRef.current = null;
    if (!recognition) return;
    try {
      recognition.stop();
    } catch (error) {
      setVoiceError(readErrorMessage(t, error));
    }
  }

  const toggleLocalVoiceInput = async () => {
    if (voiceMode === "recording") {
      stopLocalVoiceRecording(true);
      return;
    }
    if (voiceMode === "transcribing") return;

    if (!preferences.voiceInputEnabled) {
      setVoiceError(t("voice.error.disabled"));
      return;
    }

    if (!voiceStatus?.available) {
      setVoiceError(voiceStatus?.detail ?? t("voice.error.localSttNotConfigured"));
      return;
    }

    const speechWindow = window as WindowWithSpeech;
    const Recorder = speechWindow.MediaRecorder;
    if (!Recorder || !navigator.mediaDevices?.getUserMedia) {
      setVoiceError(t("voice.error.localRecordingUnavailable"));
      return;
    }

    try {
      setVoiceError(null);
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const mimeType = preferredRecordingMimeType(Recorder);
      const recorder = new Recorder(stream, mimeType ? { mimeType } : undefined);
      recordedChunksRef.current = [];
      shouldTranscribeRecordingRef.current = true;
      recorder.ondataavailable = (event) => {
        if (event.data.size > 0) recordedChunksRef.current.push(event.data);
      };
      recorder.onerror = (event) => {
        setVoiceError(readErrorMessage(t, (event as Event & { error?: Error }).error ?? event));
        stopLocalVoiceRecording(false);
      };
      recorder.onstop = () => {
        const shouldTranscribe = shouldTranscribeRecordingRef.current;
        const recordedMimeType = recorder.mimeType || mimeType || "audio/webm";
        cleanupLocalRecording();
        if (shouldTranscribe) void transcribeLocalRecording(recordedMimeType);
      };
      mediaStreamRef.current = stream;
      mediaRecorderRef.current = recorder;
      recorder.start();
      setListening(true);
      setVoiceMode("recording");
    } catch (error) {
      cleanupLocalRecording();
      setListening(false);
      setVoiceMode("idle");
      setVoiceError(readErrorMessage(t, error));
    }
  };

  const transcribeLocalRecording = async (mimeType: string) => {
    const chunks = recordedChunksRef.current;
    recordedChunksRef.current = [];
    if (chunks.length === 0) {
      setVoiceMode("idle");
      setListening(false);
      setVoiceError(t("voice.error.noRecordedAudio"));
      return;
    }

    try {
      setVoiceMode("transcribing");
      setListening(false);
      const blob = new Blob(chunks, { type: mimeType });
      const audioBase64 = await blobToBase64(blob);
      const result = await luxCommands.voiceTranscribeLocal({
        provider: "local",
        audioBase64,
        mimeType: blob.type || mimeType,
        language: preferences.voiceInputLanguage === "auto" ? null : preferences.voiceInputLanguage,
        command: preferences.localSttCommand || null,
        modelPath: preferences.localSttModelPath || null,
      });
      appendTranscript(result.text);
      setVoiceError(null);
    } catch (error) {
      setVoiceError(readErrorMessage(t, error));
    } finally {
      setVoiceMode("idle");
    }
  };

  function stopLocalVoiceRecording(transcribe: boolean) {
    shouldTranscribeRecordingRef.current = transcribe;
    const recorder = mediaRecorderRef.current;
    if (recorder && recorder.state !== "inactive") {
      recorder.stop();
      return;
    }
    cleanupLocalRecording();
    if (!transcribe) recordedChunksRef.current = [];
    setListening(false);
    setVoiceMode("idle");
  }

  function cleanupLocalRecording() {
    mediaRecorderRef.current = null;
    mediaStreamRef.current?.getTracks().forEach((track) => track.stop());
    mediaStreamRef.current = null;
  }

  return {
    canUseVoice,
    listening,
    toggleVoiceInput,
    voiceError,
    voiceLabel,
    voiceMode,
    voiceTitle,
  };
}

function resolveVoiceLanguage(language: AiVoiceInputLanguage) {
  if (language !== "auto") return language;
  return navigator.language || "ru-RU";
}

function formatVoiceError(t: TranslateFn, event: SpeechRecognitionErrorEventLike) {
  if (event.error === "not-allowed" || event.error === "service-not-allowed") return t("voice.error.microphonePermissionDenied");
  if (event.error === "no-speech") return t("voice.error.noSpeechDetected");
  if (event.error === "audio-capture") return t("voice.error.microphoneUnavailable");
  if (event.error === "network") return t("voice.error.serviceUnavailable");
  return event.message || event.error || t("voice.error.failed");
}

function voiceStatusLabel(t: TranslateFn, { enabled, error, listening, mode, provider, selectedProvider, status }: { enabled: boolean; error: string | null; listening: boolean; mode: VoiceInputMode; provider: "native" | "webkit" | null; selectedProvider: "native-webview" | "local"; status: VoiceInputProviderStatus | null }) {
  if (!enabled) return t("voice.status.off");
  if (mode === "transcribing") return t("voice.status.transcribing");
  if (listening) return t("voice.status.listening");
  if (error) return error;
  if (selectedProvider === "local") return status?.available ? t("voice.status.localReady") : status?.detail ?? t("voice.status.localUnavailable");
  if (!provider) return t("voice.status.unavailable");
  return provider === "native" ? t("voice.status.native") : t("voice.status.webkit");
}

function voiceButtonTitle(t: TranslateFn, { enabled, error, mode, provider, selectedProvider, status, voiceAvailable }: { enabled: boolean; error: string | null; mode: VoiceInputMode; provider: "native" | "webkit" | null; selectedProvider: "native-webview" | "local"; status: VoiceInputProviderStatus | null; voiceAvailable: boolean }) {
  if (!enabled) return t("voice.title.disabled");
  if (mode === "recording") return t("voice.title.stopAndTranscribe");
  if (mode === "transcribing") return t("voice.title.transcribingLocal");
  if (selectedProvider === "local") return status?.available ? t("voice.title.recordLocalStt") : status?.detail ?? t("voice.title.localSttUnavailable");
  if (!voiceAvailable || !provider) return t("voice.title.unavailable");
  return error ? t("voice.title.withError", { error }) : t("voice.title.default");
}

function preferredRecordingMimeType(Recorder: WindowWithSpeech["MediaRecorder"]) {
  const supported = Recorder?.isTypeSupported;
  if (!supported) return "";
  return [
    "audio/webm;codecs=opus",
    "audio/webm",
    "audio/ogg;codecs=opus",
    "audio/ogg",
    "audio/mp4",
  ].find((mimeType) => supported(mimeType)) ?? "";
}

async function blobToBase64(blob: Blob) {
  const buffer = await blob.arrayBuffer();
  let binary = "";
  const bytes = new Uint8Array(buffer);
  const chunkSize = 0x8000;
  for (let index = 0; index < bytes.length; index += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunkSize));
  }
  return btoa(binary);
}

function readErrorMessage(t: TranslateFn, error: unknown) {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return t("voice.error.failed");
}
