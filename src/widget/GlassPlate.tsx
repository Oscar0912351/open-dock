import type { CSSProperties, Ref } from "react";
import { toAssetUrl } from "../shared/ipc";

interface GlassPlateProps {
  plateRef?: Ref<HTMLDivElement>;
  platePath: string;
  monitorLogicalW: number;
  monitorLogicalH: number;
  wx: number;
  wy: number;
}

export function GlassPlate({
  plateRef,
  platePath,
  monitorLogicalW,
  monitorLogicalH,
  wx,
  wy,
}: GlassPlateProps) {
  const plateUrl = toAssetUrl(platePath);
  if (!plateUrl) {
    return <div ref={plateRef} className="glass-plate" aria-hidden="true" />;
  }

  const style = {
    "--plate": `url("${plateUrl}")`,
    "--mon-w": `${monitorLogicalW}px`,
    "--mon-h": `${monitorLogicalH}px`,
    "--wx": `${wx}px`,
    "--wy": `${wy}px`,
  } as CSSProperties;

  return <div ref={plateRef} className="glass-plate" style={style} aria-hidden="true" />;
}
