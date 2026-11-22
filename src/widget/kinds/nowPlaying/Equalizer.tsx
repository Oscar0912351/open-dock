import { useEffect, useState, type CSSProperties } from "react";

interface EqualizerProps {
  playing: boolean;
  height?: number;
  className?: string;
}

const PAUSED_HEIGHTS = [0.3, 0.55, 0.4, 0.25] as const;
const PLAYING_FRAMES: ReadonlyArray<readonly [number, number, number, number]> = [
  [0.36, 0.92, 0.56, 0.78],
  [0.82, 0.42, 0.96, 0.36],
  [0.52, 0.76, 0.34, 0.9],
  [0.94, 0.38, 0.72, 0.48],
  [0.44, 0.88, 0.62, 0.32],
  [0.74, 0.54, 0.92, 0.68],
];
const STEP_MS = 650;

export function Equalizer({ playing, height = 13, className = "" }: EqualizerProps) {
  const [frame, setFrame] = useState(0);

  useEffect(() => {
    if (!playing) return;
    if (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) return;
    const id = window.setInterval(() => {
      if (document.hidden) return;
      setFrame((prev) => (prev + 1) % PLAYING_FRAMES.length);
    }, STEP_MS);
    return () => window.clearInterval(id);
  }, [playing]);

  const heights = playing ? (PLAYING_FRAMES[frame] ?? PAUSED_HEIGHTS) : PAUSED_HEIGHTS;
  const wrapStyle: CSSProperties & Record<string, string | number> = { "--eqh": height };
  return (
    <div
      className={`eq${playing ? "" : " paused"}${className ? ` ${className}` : ""}`}
      style={wrapStyle}
      aria-hidden="true"
    >
      {heights.map((h, index) => (
        <i
          key={index}
          style={{ "--h": h } as CSSProperties}
        />
      ))}
    </div>
  );
}
