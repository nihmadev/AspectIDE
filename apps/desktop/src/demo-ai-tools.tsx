import { createRoot } from "react-dom/client";
import { AspectorToolsPanel } from "./components/Aspector/AspectorToolsPanel";
import "./styles/tokens.css";
import "./styles/ai-tools.css";

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(<AspectorToolsPanel />);
}
