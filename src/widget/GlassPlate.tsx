import type { CSSProperties, Ref } from "react";
import { toAssetUrl } from "../shared/ipc";

interface GlassPlateProps {
  plateRef?: Ref<HTMLDivElement>;
  platePath: string;
  glassPlatePath?: string;
  preblurred?: boolean;
  monitorLogicalW: number;
  monitorLogicalH: number;
  wx: number;
  wy: number;
}

export function GlassPlate({
  plateRef,
  platePath,
  glassPlatePath,
  preblurred = false,
  monitorLogicalW,
  monitorLogicalH,
  wx,
  wy,
}: GlassPlateProps) {
  const usePreblurred = Boolean(preblurred && glassPlatePath);
  const plateUrl = toAssetUrl(usePreblurred ? (glassPlatePath ?? "") : platePath);
  const className = usePreblurred ? "glass-plate is-preblurred" : "glass-plate";
  if (!plateUrl) {
    return <div ref={plateRef} className={className} aria-hidden="true" />;
  }

  const style = {
    "--plate": `url("${plateUrl}")`,
    "--mon-w": `${monitorLogicalW}px`,
    "--mon-h": `${monitorLogicalH}px`,
    "--wx": `${wx}px`,
    "--wy": `${wy}px`,
  } as CSSProperties;

  return <div ref={plateRef} className={className} style={style} aria-hidden="true" />;
}
