import logoUrl from "../assets/open-dock-logo.png";

interface DockLogoProps {
  size?: number;
  animated?: boolean;
}

export function DockLogo({ size = 18, animated = false }: DockLogoProps) {
  return (
    <img
      src={logoUrl}
      width={size}
      height={size}
      alt=""
      aria-hidden="true"
      draggable={false}
      className={animated ? "od-logo-img is-animated" : "od-logo-img"}
    />
  );
}
