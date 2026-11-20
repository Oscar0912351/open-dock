import { WarningCircle } from "@phosphor-icons/react";
import type { CSSProperties, ReactNode } from "react";
import { t } from "../../shared/i18n";
import "./states.css";

interface StateEmptyProps {
  icon: ReactNode;
  title: string;
  subtitle?: string;
  compact?: boolean;
}

export function StateEmpty({ icon, title, subtitle, compact = false }: StateEmptyProps) {
  if (compact) {
    return (
      <div className="state-empty-pill" role="status">
        <span className="state-empty-pill-icon ink3" aria-hidden="true">
          {icon}
        </span>
        <div className="state-empty-pill-text col">
          <span className="title-s">{title}</span>
          {subtitle && <span className="micro">{subtitle}</span>}
        </div>
      </div>
    );
  }

  return (
    <div className="state-empty col" role="status">
      <span className="state-empty-icon ink3" data-enter aria-hidden="true">
        {icon}
      </span>
      <span className="state-empty-title title-s" data-enter>
        {title}
      </span>
      {subtitle && (
        <span className="state-empty-sub meta" data-enter>
          {subtitle}
        </span>
      )}
    </div>
  );
}

interface StateErrorProps {
  header?: ReactNode;
  title?: string;
  subtitle?: string;
}

export function StateError({
  header,
  title = t("state.error"),
  subtitle = t("state.retrySoon"),
}: StateErrorProps) {
  return (
    <div className="state-error col" role="alert">
      {header && <div className="state-error-header">{header}</div>}
      <div className="state-error-body col grow">
        <span className="state-error-icon ink3" data-enter aria-hidden="true">
          <WarningCircle size="calc(18 * var(--u))" />
        </span>
        <span className="state-error-title meta" data-enter>
          {title}
        </span>
        <span className="micro" data-enter>
          {subtitle}
        </span>
      </div>
    </div>
  );
}

interface SkeletonProps {
  width?: number | string;
  height?: number | string;
  radius?: number | string;
  circle?: boolean;
  className?: string;
}

function toUnitSize(value: number | string): string {
  return typeof value === "number" ? `calc(${value} * var(--u))` : value;
}

export function Skeleton({
  width = "100%",
  height = 12,
  radius = 6,
  circle = false,
  className = "",
}: SkeletonProps) {
  const style: CSSProperties = {
    width: toUnitSize(width),
    height: toUnitSize(height),
    borderRadius: circle ? "50%" : toUnitSize(radius),
  };

  return <div className={`sk${className ? ` ${className}` : ""}`} style={style} aria-hidden="true" />;
}

interface StaleIndicatorProps {
  label: string;
}

export function StaleIndicator({ label }: StaleIndicatorProps) {
  return (
    <div className="micro stale" data-enter>
      {label}
    </div>
  );
}
