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
  meta,
  stacked = false,
  children,
}: {
  title: string;
  help?: string;
  meta?: ReactNode;
  stacked?: boolean;
  children?: ReactNode;
}) {
  return (
    <div className={`cc-setting-row${stacked ? " is-stacked" : ""}`}>
      <div className="cc-setting-text">
        <div className="cc-setting-head">
          <span className="cc-setting-title">{title}</span>
          {meta !== undefined && <span className="cc-setting-meta">{meta}</span>}
        </div>
        {help && <span className="cc-setting-help">{help}</span>}
      </div>
      {children !== undefined && <div>{children}</div>}
    </div>
  );
}
