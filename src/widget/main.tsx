import React from "react";
import ReactDOM from "react-dom/client";
import "../shared/theme.css";
import { installDevtoolsShortcut, installErrorReporting } from "../shared/runtime";
import { WidgetApp } from "./App";

installErrorReporting("widget");
installDevtoolsShortcut();
const id = new URLSearchParams(window.location.search).get("id") ?? "";
const root = document.getElementById("root");

if (root) {
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <WidgetApp id={id} />
    </React.StrictMode>,
  );
}
