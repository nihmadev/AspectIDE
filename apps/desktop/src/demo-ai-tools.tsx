import { createRoot } from "react-dom/client";
import { AiToolsPanel } from "./components/AiToolsPanel";
import "./styles/tokens.css";
import "./styles/ai-tools.css";

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(<AiToolsPanel />);
}
