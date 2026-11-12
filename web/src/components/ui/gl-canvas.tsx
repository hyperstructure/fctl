import { mount, type Scene, type StageOptions } from "@hyperstructure/design/gl/core";
import { useEffect, useRef } from "react";
import { watchWebgl } from "@/lib/app-log";
import { cn } from "@/lib/cn";

/**
 * Mounts a WebGL scene on a canvas filling its parent. The scene factory and
 * options are read once, at mount; scenes read changing state through refs.
 */
export function GLCanvas({
  name,
  scene,
  options,
  className,
}: {
  /** What the scene is, for the app's log (lib/app-log.ts): `Agent Floor's scene`. */
  name: string;
  scene: () => Scene;
  options?: StageOptions;
  className?: string;
}) {
  const ref = useRef<HTMLCanvasElement>(null);
  const init = useRef({ scene, options, name });
  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const unmount = mount(canvas, init.current.scene(), init.current.options);
    const unwatch = watchWebgl(canvas, init.current.name);
    return () => {
      unmount();
      unwatch();
    };
  }, []);
  return <canvas ref={ref} className={cn("pointer-events-none block size-full", className)} />;
}
