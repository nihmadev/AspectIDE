import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { useEffect, useRef } from "react";
import { useLuxStore } from "../lib/store";
import { isBrowserPreviewRuntime, isTauriRuntime, luxCommands, subscribeLuxEvents } from "../lib/tauri";
import type { TerminalSessionInfo } from "../lib/types";
import "@xterm/xterm/css/xterm.css";

const webPrompt = "$ ";

type XtermTerminalProps = {
  bufferText?: string;
  clearToken: number;
  session: TerminalSessionInfo | null;
  onSessionCreated?: (session: TerminalSessionInfo) => void;
};

export function XtermTerminal({ bufferText = "", clearToken, onSessionCreated, session }: XtermTerminalProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const fitFrameRef = useRef<number | null>(null);
  const sessionRef = useRef<TerminalSessionInfo | null>(session);
  const bufferTextRef = useRef(bufferText);
  const renderedSessionIdRef = useRef<string | null>(null);
  const webPromptWrittenRef = useRef(false);
  const upsertTerminalSession = useLuxStore((state) => state.upsertTerminalSession);
  const appendTerminalOutput = useLuxStore((state) => state.appendTerminalOutput);

  useEffect(() => {
    sessionRef.current = session;
  }, [session]);

  useEffect(() => {
    bufferTextRef.current = bufferText;
  }, [bufferText]);

  useEffect(() => {
    const terminal = terminalRef.current;
    const sessionId = session?.id ?? null;
    if (!terminal || renderedSessionIdRef.current === sessionId) return;
    renderedSessionIdRef.current = sessionId;
    webPromptWrittenRef.current = false;
    terminal.clear();
    if (bufferText) terminal.write(bufferText);
    else if (!sessionId && isBrowserPreviewRuntime()) {
      terminal.write(webPrompt);
      webPromptWrittenRef.current = true;
    }
  }, [bufferText, session?.id]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container || terminalRef.current) return;

    const terminal = new Terminal({
      allowProposedApi: false,
      convertEol: true,
      cursorBlink: true,
      cursorStyle: "bar",
      fontFamily: "Consolas, Cascadia Mono, Courier New, monospace",
      fontSize: 13,
      lineHeight: 1.35,
      scrollback: 10_000,
      theme: {
        background: "#181818",
        foreground: "#cccccc",
        cursor: "#cccccc",
        selectionBackground: "#264f78",
        black: "#000000",
        red: "#cd3131",
        green: "#0dbc79",
        yellow: "#e5e510",
        blue: "#2472c8",
        magenta: "#bc3fbc",
        cyan: "#11a8cd",
        white: "#e5e5e5",
        brightBlack: "#666666",
        brightRed: "#f14c4c",
        brightGreen: "#23d18b",
        brightYellow: "#f5f543",
        brightBlue: "#3b8eea",
        brightMagenta: "#d670d6",
        brightCyan: "#29b8db",
        brightWhite: "#ffffff",
      },
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(container);

    terminalRef.current = terminal;
    fitRef.current = fitAddon;
    renderedSessionIdRef.current = sessionRef.current?.id ?? null;
    if (bufferTextRef.current) terminal.write(bufferTextRef.current);
    else if (!sessionRef.current && isBrowserPreviewRuntime()) {
      terminal.write(webPrompt);
      webPromptWrittenRef.current = true;
    }

    const scheduleFit = () => {
      if (fitFrameRef.current !== null) return;
      fitFrameRef.current = window.requestAnimationFrame(() => {
        fitFrameRef.current = null;
        if (!container.isConnected) return;
        const rect = container.getBoundingClientRect();
        if (rect.width <= 0 || rect.height <= 0) return;

        const previousCols = terminal.cols;
        const previousRows = terminal.rows;
        fitAddon.fit();

        const activeSession = sessionRef.current;
        if (activeSession && (terminal.cols !== previousCols || terminal.rows !== previousRows)) {
          void luxCommands.terminalResize(activeSession.id, terminal.cols, terminal.rows);
        }
      });
    };

    scheduleFit();

    terminal.onData((data) => {
      const activeSession = sessionRef.current;
      if (activeSession && isTauriRuntime()) {
        void luxCommands.terminalWrite(activeSession.id, data);
        return;
      }
      if (!isBrowserPreviewRuntime()) return;
      if (activeSession) appendTerminalOutput(activeSession.id, data);
      terminal.write(data === "\r" ? `\r\n${webPrompt}` : data);
    });

    const observer = new ResizeObserver(() => {
      scheduleFit();
    });
    observer.observe(container);

    return () => {
      if (fitFrameRef.current !== null) {
        window.cancelAnimationFrame(fitFrameRef.current);
        fitFrameRef.current = null;
      }
      observer.disconnect();
      terminal.dispose();
      terminalRef.current = null;
      fitRef.current = null;
      sessionRef.current = null;
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    const ensureSession = async () => {
      const terminal = terminalRef.current;
      if (!terminal) return;
      if (session) return;
      if (isBrowserPreviewRuntime()) {
        if (!webPromptWrittenRef.current) {
          terminal.write(webPrompt);
          webPromptWrittenRef.current = true;
        }
        return;
      }
      const created = await luxCommands.terminalCreate(undefined, undefined, terminal.cols, terminal.rows);
      if (!disposed) {
        upsertTerminalSession(created, true);
        onSessionCreated?.(created);
      } else {
        void luxCommands.terminalClose(created.id).catch(() => undefined);
      }
    };
    void ensureSession();
    return () => {
      disposed = true;
    };
  }, [onSessionCreated, session, upsertTerminalSession]);

  useEffect(() => {
    let dispose: (() => void) | undefined;
    void subscribeLuxEvents((event) => {
      if (event.type !== "terminalOutput") return;
      if (event.session_id !== sessionRef.current?.id) return;
      terminalRef.current?.write(event.data);
    }).then((unlisten) => {
      dispose = unlisten;
    }).catch((error: unknown) => {
      terminalRef.current?.write(`\r\n${readErrorMessage(error)}\r\n`);
    });

    return () => dispose?.();
  }, []);

  useEffect(() => {
    if (clearToken > 0) {
      terminalRef.current?.clear();
      if (!sessionRef.current && isBrowserPreviewRuntime()) {
        terminalRef.current?.write(webPrompt);
        webPromptWrittenRef.current = true;
      }
    }
  }, [clearToken]);

  return <div className="xterm-host" ref={containerRef} />;
}

function readErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
