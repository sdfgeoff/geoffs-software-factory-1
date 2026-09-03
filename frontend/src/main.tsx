import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

function App() {
  return (
    <main>
      <p className="eyebrow">LOCAL WORKSPACE</p>
      <h1>Multi-agent board</h1>
      <p className="lede">Coordinate persistent Codex agents from one place.</p>
    </main>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

