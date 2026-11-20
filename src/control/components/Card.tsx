import type { ReactNode } from "react";

export function Card({
  children,
  compact = false,
  className = "",
}: {
  children: ReactNode;
  compact?: boolean;
  className?: string;
}) {
  return (
    <div className={`cc-card${compact ? " is-compact" : ""}${className ? ` ${className}` : ""}`}>
      {children}
    </div>
  );
}

export function SettingRow({
  title,
  help,
  stacked = false,
  children,
}: {
  title: string;
  help?: string;
  stacked?: boolean;
  children?: ReactNode;
}) {
  return (
    <div className={`cc-setting-row${stacked ? " is-stacked" : ""}`}>
      <div className="cc-setting-text">
        <span className="cc-setting-title">{title}</span>
        {help && <span className="cc-setting-help">{help}</span>}
      </div>
      {children !== undefined && <div>{children}</div>}
    </div>
  );
}
