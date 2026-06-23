import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { useEffect, useRef } from "react";
import { useLuxStore } from "../lib/store";
import { isBrowserPreviewRuntime, isTauriRuntime, luxCommands } from "../lib/tauri";
import type { TerminalSessionInfo } from "../lib/types";
import "@xterm/xterm/css/xterm.css";

const webPrompt = "$ ";

type XtermTerminalProps = {
  /**
   * The FULL accumulated output buffer for this session. The always-on global
   * terminalOutput listener (App.tsx) appends every PTY chunk into the store
   * buffer synchronously, so this prop captures 100% of the output with no race.
   * We render it incrementally (delta writes) — this is what makes a brand-new
   * terminal show its shell prompt instead of staying blank.
   */
  bufferText?: string;
  clearToken: number;
  session: TerminalSessionInfo | null;
};

export function XtermTerminal({ bufferText = "", clearToken, session }: XtermTerminalProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const fitFrameRef = useRef<number | null>(null);
  const sessionRef = useRef<TerminalSessionInfo | null>(session);
  // How many chars of bufferText have already been written to the canvas.
  const writtenLenRef = useRef(0);
  const webPromptWrittenRef = useRef(false);
  const appendTerminalOutput = useLuxStore((state) => state.appendTerminalOutput);

  useEffect(() => {
    sessionRef.current = session;
  }, [session]);

  // Create the xterm instance once. The instance is kept alive for the lifetime
  // of this session's slot (per-session component), so switching tabs/sessions
  // never disposes it and never needs a raw-byte replay.
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

    // Paint whatever output already exists for this session (e.g. created just
    // before this instance mounted), then track from there.
    if (bufferText) {
      terminal.write(bufferText);
      writtenLenRef.current = bufferText.length;
    } else if (!session && isBrowserPreviewRuntime()) {
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
    // Focus so typing works immediately after open.
    terminal.focus();

    terminal.onData((data) => {
      const activeSession = sessionRef.current;
      if (activeSession && isTauriRuntime()) {
        // Write keystrokes to the PTY; the shell echoes them back through the
        // global terminalOutput listener → store buffer → delta write below.
        void luxCommands.terminalWrite(activeSession.id, data).catch(() => undefined);
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
      // Do NOT close the PTY here — the shell must outlive an unmount (tab switch /
      // panel hide). It is closed explicitly via the close button or workspace close.
      terminal.dispose();
      terminalRef.current = null;
      fitRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Incrementally paint new output: write only the suffix added since last time.
  // This is the single display path — no async event subscription to race with.
  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    const written = writtenLenRef.current;
    if (bufferText.length === written) return;
    if (bufferText.length > written && bufferText.startsWith(bufferText.slice(0, written))) {
      // Normal append — write the delta.
      terminal.write(bufferText.slice(written));
      writtenLenRef.current = bufferText.length;
    } else {
      // Buffer shrank or diverged (clear / truncation) — repaint from scratch.
      terminal.clear();
      if (bufferText) terminal.write(bufferText);
      writtenLenRef.current = bufferText.length;
    }
  }, [bufferText]);

  // Explicit clear (the broom button bumps clearToken).
  useEffect(() => {
    if (clearToken > 0) {
      terminalRef.current?.clear();
      writtenLenRef.current = 0;
      if (!sessionRef.current && isBrowserPreviewRuntime()) {
        terminalRef.current?.write(webPrompt);
        webPromptWrittenRef.current = true;
      }
    }
  }, [clearToken]);

  // Refocus when this slot becomes the active session so the user can type right away.
  useEffect(() => {
    if (session) terminalRef.current?.focus();
  }, [session?.id]);

  return <div className="xterm-host" ref={containerRef} data-session-id={session?.id} />;
}
