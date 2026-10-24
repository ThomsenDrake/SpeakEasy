import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { LayoutProvider } from "@once-ui-system/core/contexts";

import App from "./App";

import "./styles/globals.css";

// Apply the Once UI theme attributes before first paint. The Tauri CSP
// forbids inline scripts, so the theme is set from this entry module rather
// than a <script> in index.html. The attributes mirror Once UI's defaults
// (neutral gray, brand blue, accent indigo, contrast solids, rounded
// corners, filled surfaces); `data-theme` follows the system appearance.
// `?theme=light|dark` overrides it so both themes can be inspected under
// `vite preview`.
const params = new URLSearchParams(window.location.search);
const forcedTheme = params.get("theme");
const dark =
  forcedTheme === "dark" ||
  (forcedTheme !== "light" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches);
const root = document.documentElement;
root.dataset.theme = dark ? "dark" : "light";
root.dataset.neutral = "gray";
root.dataset.brand = "blue";
root.dataset.accent = "indigo";
root.dataset.solid = "contrast";
root.dataset.border = "rounded";
root.dataset.surface = "filled";
root.dataset.transition = "all";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {/* Once UI's client-side layout primitives (used internally by Tag,
        Textarea, …) read breakpoint state from this context. The root
        LayoutProvider renders plain <a>/<img> adapters, which is what a
        non-Next app wants. */}
    <LayoutProvider>
      <App />
    </LayoutProvider>
  </StrictMode>,
);
