import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource-variable/schibsted-grotesk";
import "./theme.css";
import App from "./App";

// This is a desktop app, not a browser: the WebView's own right-click menu
// (Reload / Back / Inspect) has no place here and only gets in the way.
window.addEventListener("contextmenu", e => e.preventDefault());

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
