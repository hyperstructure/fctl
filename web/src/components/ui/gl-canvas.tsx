import { useEffect, useRef } from "react";
import { mount, type Scene, type StageOptions } from "@/gl/core";
import { cn } from "@/lib/cn";

/**
 * Mounts a WebGL scene on a canvas filling its parent. The scene factory and
 * options are read once, at mount; scenes read changing state through refs.
 */
export function GLCanvas({
  scene,
  options,
  className,
}: {
  scene: () => Scene;
  options?: StageOptions;
  className?: string;
}) {
  const ref = useRef<HTMLCanvasElement>(null);
  const init = useRef({ scene, options });
  useEffect(() => {
    if (!ref.current) return;
    return mount(ref.current, init.current.scene(), init.current.options);
  }, []);
  return <canvas ref={ref} className={cn("pointer-events-none block size-full", className)} />;
}
