import { mount } from "@hyperstructure/design/gl/core";
import { play, type StreamHandle, stream } from "@hyperstructure/design/gl/stream";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { Logo } from "@/components/ui";
import { cn } from "@/lib/cn";

/**
 * The not-found and error pages: the code (404, 503…) as glyphs in the
 * stream, the scene from @hyperstructure/design where the foundry's threads
 * flow around them, with the page's copy below. People can stir the threads,
 * throw the glyphs and tap for a burst. With `power` off the wind and the
 * lights go out and the glyphs fall; they stand up again when it comes back.
 * Without WebGL the code shows as text.
 */
export function StreamScreen({
  code,
  power = true,
  children,
}: {
  code: string;
  /** Whether the wind blows. A page that starts without it shows the glyphs standing first. */
  power?: boolean;
  children: ReactNode;
}) {
  const root = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const box = useRef<HTMLDivElement>(null);
  const scene = useRef<StreamHandle | null>(null);
  const [live, setLive] = useState(false);
  const [gl, setGl] = useState(true);

  useEffect(() => {
    const el = root.current;
    const c = canvas.current;
    const b = box.current;
    if (!el || !c || !b) return;
    const s = stream({ text: code });
    const unmount = mount(c, s.scene, { pixel: 3 });
    if (c.dataset.webgl) {
      setGl(false);
      return unmount;
    }
    const place = () => {
      const cr = c.getBoundingClientRect();
      const br = b.getBoundingClientRect();
      if (!cr.width || !cr.height) return;
      s.place({
        x: (br.left - cr.left) / cr.width,
        y: (br.top - cr.top) / cr.height,
        width: br.width / cr.width,
        height: br.height / cr.height,
      });
    };
    const ro = new ResizeObserver(place);
    ro.observe(el);
    ro.observe(b);
    place();
    const unplay = play(el, c, s);
    let alive = true;
    void s.ready.then(() => {
      if (!alive) return;
      scene.current = s;
      setLive(true);
    });
    return () => {
      alive = false;
      scene.current = null;
      setLive(false);
      ro.disconnect();
      unplay();
      unmount();
    };
  }, [code]);

  // The power follows the prop; the first time it is off, after a moment.
  const first = useRef(true);
  useEffect(() => {
    const s = scene.current;
    if (!live || !s) return;
    if (!first.current) {
      s.power(power);
      return;
    }
    first.current = false;
    if (power) return;
    const t = setTimeout(() => s.power(false), 700);
    return () => clearTimeout(t);
  }, [live, power]);

  return (
    <div ref={root} className={cn("stream-screen", live && "live")}>
      <canvas ref={canvas} className="stream-screen-canvas" />
      <div className="stream-screen-shade" aria-hidden="true" />
      <a
        href="/"
        className="absolute top-5 left-5 z-10 rounded-md sm:top-6 sm:left-7"
        aria-label="fctl, the floor"
      >
        <Logo />
      </a>
      <div ref={box} className={cn("stream-screen-code", !gl && "visible")} aria-hidden="true">
        {code}
      </div>
      <div className="stream-screen-copy">{children}</div>
      <p className="stream-screen-hint" aria-hidden="true">
        The {/^\d+$/.test(code) ? "numbers" : "letters"} can be picked up and thrown.
      </p>
    </div>
  );
}
