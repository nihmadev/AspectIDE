// Example: How to integrate AiToolsPanel into the main app

import { AspectorToolsPanel } from "./components/Aspector/AspectorToolsPanel";

// Option 1: As a standalone view (like WelcomeScreen)
// Replace the editor area content with the tools panel
function App() {
  return (
    <div className="app-shell">
      <TitleBar />
      <div className="workbench">
        <AiToolsPanel />
      </div>
      <StatusBar />
    </div>
  );
}

// Option 2: As a sidebar panel (like Explorer)
// Add to the sidebar activities
const activities = [
  { id: "explorer", labelKey: "activity.explorer", icon: Files },
  { id: "ai-tools", labelKey: "activity.aiTools", icon: Wrench }, // New!
  // ... other activities
];

// Then in Sidebar.tsx, add a case for "ai-tools":
function Sidebar() {
  const activeActivity = useLuxStore((state) => state.activeActivity);

  return (
    <aside className="sidebar">
      {activeActivity === "explorer" && <ExplorerPanel />}
      {activeActivity === "ai-tools" && <AiToolsPanel />}
      {/* ... other panels */}
    </aside>
  );
}

// Option 3: As a right-side panel (like AI Chat)
// Add state to store.ts:
// aiToolsOpen: boolean
// toggleAiTools: () => void

// Then in App.tsx, render conditionally:
<Group orientation="horizontal" className="main-panels">
  <Panel minSize="360px">
    <EditorArea />
  </Panel>
  {aiToolsOpen && (
    <>
      <Separator className="resize-handle editor-group-separator" />
      <Panel defaultSize="32%" minSize="300px" maxSize="48%">
        <AiToolsPanel />
      </Panel>
    </>
  )}
</Group>

// Option 4: As a modal/dialog
// Trigger from command palette or keyboard shortcut
function showAiToolsDialog() {
  // Render AiToolsPanel in a dialog overlay
  return (
    <div className="command-overlay">
      <div className="settings-dialog" style={{ maxWidth: "1200px" }}>
        <AiToolsPanel />
      </div>
    </div>
  );
}

export {};
