import React from "react";
import ReactDOM from "react-dom/client";
import "../shared/theme.css";
import "./control.css";
import { installDevtoolsShortcut, installErrorReporting } from "../shared/runtime";
import { App } from "./App";

installErrorReporting("control");
installDevtoolsShortcut();
const root = document.getElementById("root");

if (root) {
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}
