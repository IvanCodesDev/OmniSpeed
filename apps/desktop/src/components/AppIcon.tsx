import { Play } from "lucide-react";
import chromeSvg from "../assets/brands/chrome.svg";
import edgeSvg from "../assets/brands/edge.svg";
import potplayerSvg from "../assets/brands/potplayer.svg";
import vlcSvg from "../assets/brands/vlc.svg";
import type { BrandId } from "../data";
import { cn } from "../lib/cn";

const brandFiles: Record<Exclude<BrandId, "unknown">, string> = {
  chrome: chromeSvg,
  edge: edgeSvg,
  vlc: vlcSvg,
  potplayer: potplayerSvg,
};

export function AppIcon({
  id,
  size = 34,
  className,
}: {
  id: BrandId;
  size?: number;
  className?: string;
}) {
  if (id === "unknown") {
    return (
      <span
        className={cn(
          "grid shrink-0 place-items-center rounded-[10px] border border-line bg-card-2 text-mute",
          className,
        )}
        style={{ width: size, height: size }}
      >
        <Play size={size * 0.42} />
      </span>
    );
  }
  return (
    <img
      src={brandFiles[id]}
      width={size}
      height={size}
      className={cn("shrink-0 select-none", className)}
      draggable={false}
      alt=""
    />
  );
}
