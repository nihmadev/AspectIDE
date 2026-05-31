import { Minus, PanelLeft, PanelTop, Settings, Sparkles, Square, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { PointerEvent as ReactPointerEvent } from "react";
import { useEffect, useRef, useState } from "react";
import { useEditorCloseGuard } from "./EditorCloseGuard";
import { useTranslation } from "../lib/i18n/useTranslation";
import { closedDocumentIdsForAllDocuments } from "../lib/editorCloseTargets";
import { resetEditorFontZoom, toggleEditorMinimap, toggleEditorWordWrap, zoomEditorFontIn, zoomEditorFontOut } from "../lib/editorPreferenceCommands";
import { useLuxStore } from "../lib/store";
import { isTauriRuntime, luxCommands } from "../lib/tauri";
import { pickAndOpenWorkspace, reloadWorkspace } from "../lib/workspaceActions";

type TitleMenuItem = {
  label: string;
  shortcut?: string;
  disabled?: boolean;
  separatorBefore?: boolean;
  onClick?: () => void;
};

type TitleMenu = {
  id: string;
  label: string;
  groups: TitleMenuItem[][];
};

async function handleWindowAction(action: "minimize" | "maximize" | "close" | "destroy") {
  if (!isTauriRuntime()) return;
  const appWindow = getCurrentWindow();
  if (action === "minimize") await appWindow.minimize();
  if (action === "maximize") await appWindow.toggleMaximize();
  if (action === "close") await appWindow.close();
  if (action === "destroy") await appWindow.destroy();
}

function isTitleBarInteractiveTarget(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) return false;
  return Boolean(target.closest("button, a, input, textarea, select, [role='menu'], .top-menu, .mode-switcher, .title-actions, .window-controls"));
}

export function TitleBar() {
  const workspaceMode = useLuxStore((state) => state.workspaceMode);
  const setWorkspaceMode = useLuxStore((state) => state.setWorkspaceMode);
  const workspace = useLuxStore((state) => state.workspace);
  const activeDocumentId = useLuxStore((state) => state.activeDocumentId);
  const openDocuments = useLuxStore((state) => state.openDocuments);
  const sidebarVisible = useLuxStore((state) => state.sidebarVisible);
  const toggleSidebar = useLuxStore((state) => state.toggleSidebar);
  const aiChatOpen = useLuxStore((state) => state.aiChatOpen);
  const toggleAiChat = useLuxStore((state) => state.toggleAiChat);
  const bottomPanelOpen = useLuxStore((state) => state.bottomPanelOpen);
  const setBottomPanelOpen = useLuxStore((state) => state.setBottomPanelOpen);
  const editorPreferences = useLuxStore((state) => state.editorPreferences);
  const setWorkspace = useLuxStore((state) => state.setWorkspace);
  const upsertDocument = useLuxStore((state) => state.upsertDocument);
  const openBottomPanel = useLuxStore((state) => state.openBottomPanel);
  const setActiveActivity = useLuxStore((state) => state.setActiveActivity);
  const setSidebarVisible = useLuxStore((state) => state.setSidebarVisible);
  const setCommandPaletteOpen = useLuxStore((state) => state.setCommandPaletteOpen);
  const setSettingsOpen = useLuxStore((state) => state.setSettingsOpen);
  const [openMenu, setOpenMenu] = useState<string | null>(null);
  const menuRef = useRef<HTMLElement | null>(null);
  const hoverSwitchedMenuRef = useRef<string | null>(null);
  const { requestCloseDocuments } = useEditorCloseGuard();
  const { t } = useTranslation();

  const handleTitleBarPointerDown = (event: ReactPointerEvent<HTMLElement>) => {
    if (!isTauriRuntime() || event.button !== 0 || isTitleBarInteractiveTarget(event.target)) return;
    void getCurrentWindow().startDragging().catch(() => undefined);
  };

  useEffect(() => {
    if (!openMenu) return;
    const closeIfOutside = (event: PointerEvent) => {
      if (menuRef.current?.contains(event.target as Node)) return;
      setOpenMenu(null);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpenMenu(null);
    };
    window.addEventListener("pointerdown", closeIfOutside);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("pointerdown", closeIfOutside);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [openMenu]);

  const openCurrentWorkspace = () => {
    requestCloseDocuments(closedDocumentIdsForAllDocuments(openDocuments), () => {
      const action = workspace ? reloadWorkspace(workspace) : pickAndOpenWorkspace();
      void action.then((openedWorkspace) => {
        if (openedWorkspace) setWorkspace(openedWorkspace);
      }).catch(() => undefined);
    }, { title: workspace ? t("titlebar.dialog.saveBeforeReloadingFolder") : t("titlebar.dialog.saveBeforeOpeningFolder") });
  };

  const closeWorkspace = () => {
    requestCloseDocuments(closedDocumentIdsForAllDocuments(openDocuments), () => {
      void luxCommands.workspaceClose().then(() => setWorkspace(null)).catch(() => undefined);
    }, { title: t("titlebar.dialog.saveBeforeClosingFolder") });
  };

  const saveActiveEditor = () => {
    if (!activeDocumentId) return;
    void luxCommands.editorSaveFile(activeDocumentId).then(upsertDocument).catch(() => undefined);
  };

  const saveActiveEditorAs = () => {
    if (!activeDocumentId) return;
    void luxCommands.editorSaveFileAs(activeDocumentId).then(upsertDocument).catch(() => undefined);
  };

  const newUntitledFile = () => {
    void luxCommands.editorNewFile().then(upsertDocument).catch(() => undefined);
  };

  const showActivity = (activity: Parameters<typeof setActiveActivity>[0]) => {
    setActiveActivity(activity);
    setSidebarVisible(true);
  };

  const menus: TitleMenu[] = [
    {
      id: "file",
      label: t("titlebar.menu.file"),
      groups: [
        [
          { label: t("titlebar.menu.file.newTextFile"), shortcut: "Ctrl+N", onClick: newUntitledFile },
          { label: t("titlebar.menu.file.openFolder"), shortcut: "Ctrl+O", onClick: openCurrentWorkspace },
        ],
        [
          { label: t("titlebar.menu.file.save"), shortcut: "Ctrl+S", disabled: !activeDocumentId, onClick: saveActiveEditor },
          { label: t("titlebar.menu.file.saveAs"), shortcut: "Ctrl+Shift+S", disabled: !activeDocumentId, onClick: saveActiveEditorAs },
        ],
        [
          { label: t("titlebar.menu.file.preferences"), shortcut: "Ctrl+,", onClick: () => setSettingsOpen(true) },
          { label: t("titlebar.menu.file.closeFolder"), disabled: !workspace, onClick: closeWorkspace },
        ],
      ],
    },
    {
      id: "edit",
      label: t("titlebar.menu.edit"),
      groups: [
        [
          { label: t("titlebar.menu.commandPalette"), shortcut: "Ctrl+Shift+P", onClick: () => setCommandPaletteOpen(true) },
        ],
      ],
    },
    {
      id: "view",
      label: t("titlebar.menu.view"),
      groups: [
        [
          { label: t("titlebar.menu.commandPalette"), shortcut: "Ctrl+Shift+P", onClick: () => setCommandPaletteOpen(true) },
          { label: sidebarVisible ? t("titlebar.menu.view.hideSideBar") : t("titlebar.menu.view.showSideBar"), shortcut: "Ctrl+B", onClick: toggleSidebar, disabled: !workspace },
          { label: aiChatOpen ? t("titlebar.menu.view.hideChat") : t("titlebar.menu.view.showChat"), shortcut: "Ctrl+L", onClick: toggleAiChat },
          { label: bottomPanelOpen ? t("titlebar.menu.view.hidePanel") : t("titlebar.menu.view.showPanel"), onClick: () => setBottomPanelOpen(!bottomPanelOpen) },
          { label: editorPreferences.wordWrap === "on" ? t("titlebar.menu.view.disableWordWrap") : t("titlebar.menu.view.enableWordWrap"), shortcut: "Alt+Z", disabled: !activeDocumentId, onClick: toggleEditorWordWrap },
          { label: editorPreferences.minimap ? t("titlebar.menu.view.hideMinimap") : t("titlebar.menu.view.showMinimap"), shortcut: "Ctrl+M Ctrl+M", disabled: !activeDocumentId, onClick: toggleEditorMinimap },
          { label: t("titlebar.menu.view.zoomEditorFontIn"), shortcut: "Ctrl+Plus", disabled: !activeDocumentId, onClick: zoomEditorFontIn },
          { label: t("titlebar.menu.view.zoomEditorFontOut"), shortcut: "Ctrl+-", disabled: !activeDocumentId, onClick: zoomEditorFontOut },
          { label: t("titlebar.menu.view.resetEditorFontZoom"), shortcut: "Ctrl+0", disabled: !activeDocumentId, onClick: resetEditorFontZoom },
        ],
        [
          { label: t("titlebar.menu.view.explorer"), shortcut: "Ctrl+Shift+E", disabled: !workspace, onClick: () => showActivity("explorer") },
          { label: t("titlebar.menu.view.search"), shortcut: "Ctrl+Shift+F", disabled: !workspace, onClick: () => showActivity("search") },
          { label: t("titlebar.menu.view.sourceControl"), shortcut: "Ctrl+Shift+G", disabled: !workspace, onClick: () => showActivity("git") },
          { label: t("titlebar.menu.view.runAndDebug"), shortcut: "Ctrl+Shift+D", disabled: !workspace, onClick: () => showActivity("runDebug") },
          { label: t("titlebar.menu.view.extensions"), shortcut: "Ctrl+Shift+X", disabled: !workspace, onClick: () => showActivity("extensions") },
        ],
      ],
    },
    {
      id: "go",
      label: t("titlebar.menu.window"),
      groups: [
        [
          { label: t("titlebar.menu.window.goToFile"), shortcut: "Ctrl+P", onClick: () => setCommandPaletteOpen(true) },
        ],
      ],
    },
    {
      id: "help",
      label: t("titlebar.menu.help"),
      groups: [
        [
          { label: t("titlebar.menu.help.welcome"), onClick: () => setCommandPaletteOpen(true) },
        ],
      ],
    },
  ];

  return (
    <header className="title-bar" onPointerDown={handleTitleBarPointerDown}>
      <div className="title-drag-surface" aria-hidden="true" />
      <div className="title-left">
        <span className="app-cube">L</span>
        <nav className="top-menu" aria-label={t("titlebar.applicationMenu")} ref={menuRef}>
          {menus.map((menu) => (
            <div className="top-menu-item" key={menu.id}>
              <button
                className="top-menu-trigger"
                data-open={openMenu === menu.id}
                type="button"
                aria-haspopup="menu"
                aria-expanded={openMenu === menu.id}
                onMouseEnter={() => {
                  if (openMenu && openMenu !== menu.id) {
                    hoverSwitchedMenuRef.current = menu.id;
                    setOpenMenu(menu.id);
                  }
                }}
                onClick={() => setOpenMenu((current) => {
                  if (hoverSwitchedMenuRef.current === menu.id) {
                    hoverSwitchedMenuRef.current = null;
                    return menu.id;
                  }
                  return current === menu.id ? null : menu.id;
                })}
              >
                {menu.label}
              </button>
              {openMenu === menu.id && (
                <div className="top-menu-dropdown" role="menu" aria-label={menu.label}>
                  {menu.groups.map((group, groupIndex) => (
                    <div className="top-menu-group" role="group" key={`${menu.id}-${groupIndex}`}>
                      {group.map((item, itemIndex) => (
                        <button
                          className="top-menu-command"
                          type="button"
                          role="menuitem"
                          disabled={item.disabled}
                          key={`${item.label}-${itemIndex}`}
                          onClick={() => {
                            if (item.disabled) return;
                            item.onClick?.();
                            setOpenMenu(null);
                          }}
                        >
                          <span>{item.label}</span>
                          {item.shortcut ? <kbd>{item.shortcut}</kbd> : <span />}
                        </button>
                      ))}
                    </div>
                  ))}
                </div>
              )}
            </div>
          ))}
        </nav>
        <div className="mode-switcher" aria-label={t("titlebar.mode.label")}>
          <button
            type="button"
            data-active={workspaceMode === "agent"}
            aria-pressed={workspaceMode === "agent"}
            onClick={() => setWorkspaceMode("agent")}
          >
            {t("common.agent")}
          </button>
          <button
            type="button"
            data-active={workspaceMode === "workspace"}
            aria-pressed={workspaceMode === "workspace"}
            onClick={() => setWorkspaceMode("workspace")}
          >
            {t("common.workspace")}
          </button>
        </div>
      </div>
      {workspaceMode !== "agent" && workspace && (
        <div className="title-center" data-agent-mode="false">{workspace.name}</div>
      )}
      <div className="title-actions" data-agent-mode={workspaceMode === "agent"}>
        <button
          className="title-tool-button"
          type="button"
          aria-label={t("titlebar.action.toggleChat")}
          title={t("titlebar.action.toggleChat")}
          data-active={aiChatOpen}
          onClick={toggleAiChat}
        >
          <Sparkles size={15} />
        </button>
        <button
          className="title-tool-button"
          type="button"
          aria-label={t("titlebar.action.toggleBottomPanel")}
          title={t("titlebar.action.toggleBottomPanel")}
          data-active={bottomPanelOpen}
          onClick={() => setBottomPanelOpen(!bottomPanelOpen)}
        >
          <PanelTop size={15} />
        </button>
        <button
          className="title-tool-button"
          type="button"
          aria-label={t("titlebar.action.toggleSidebar")}
          title={t("titlebar.action.toggleSidebar")}
          data-active={sidebarVisible}
          disabled={!workspace}
          onClick={toggleSidebar}
        >
          <PanelLeft size={15} />
        </button>
        <button className="title-tool-button" type="button" aria-label={t("titlebar.settings")} title={t("titlebar.settings")} onClick={() => setSettingsOpen(true)}>
          <Settings size={15} />
        </button>
        <div className="window-controls">
          <button className="window-control" type="button" aria-label={t("titlebar.window.minimize")} title={t("titlebar.window.minimize")} onClick={() => void handleWindowAction("minimize")}>
            <Minus size={14} />
          </button>
          <button className="window-control" type="button" aria-label={t("titlebar.window.maximize")} title={t("titlebar.window.maximize")} onClick={() => void handleWindowAction("maximize")}>
            <Square size={12} />
          </button>
          <button
            className="window-control close"
            type="button"
            aria-label={t("titlebar.window.close")}
            title={t("titlebar.window.close")}
            onClick={() => {
              requestCloseDocuments(
                closedDocumentIdsForAllDocuments(openDocuments),
                () => void handleWindowAction("destroy"),
                { title: t("titlebar.dialog.saveBeforeClosingApp") },
              );
            }}
          >
            <X size={16} />
          </button>
        </div>
      </div>
    </header>
  );
}
