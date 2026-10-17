// A tiny WebGL2 runtime for the app's scenes.
//
// Every scene renders into its own canvas at a reduced resolution, pauses
// when the tab or the canvas is hidden, survives context loss, and renders a
// single still frame when the person prefers reduced motion.

export interface SceneContext {
  gl: WebGL2RenderingContext;
  width: number;
  height: number;
  /** Seconds since the scene started (frozen with reduced motion). */
  time: number;
  dt: number;
  dpr: number;
}

export interface Scene {
  init(gl: WebGL2RenderingContext): void;
  frame(ctx: SceneContext): void;
  dispose?(gl: WebGL2RenderingContext): void;
  /**
   * Set by mount(): call it when the scene's inputs change. It draws a new
   * still frame when motion is reduced (otherwise the next frame shows it).
   */
  invalidate?: () => void;
}

export interface StageOptions {
  /** Fraction of device pixels to render (0.5 = half resolution). */
  scale?: number;
  maxDpr?: number;
  /** Keep animating while reduced motion is requested (never for big scenes). */
  alwaysAnimate?: boolean;
  /** Frames per second cap (default 60). */
  fps?: number;
  /**
   * Render chunky: one scene pixel per this many device pixels, upscaled
   * without smoothing (for dithered scenes). Overrides `scale`.
   */
  pixel?: number;
}

export function prefersReducedMotion(): boolean {
  return typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
}

export function compile(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
  const make = (type: number, src: string) => {
    const s = gl.createShader(type);
    if (!s) throw new Error("createShader failed");
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS) && !gl.isContextLost()) {
      const log = gl.getShaderInfoLog(s);
      gl.deleteShader(s);
      throw new Error(`shader: ${log}`);
    }
    return s;
  };
  const p = gl.createProgram();
  if (!p) throw new Error("createProgram failed");
  const v = make(gl.VERTEX_SHADER, vs);
  const f = make(gl.FRAGMENT_SHADER, fs);
  gl.attachShader(p, v);
  gl.attachShader(p, f);
  gl.linkProgram(p);
  gl.deleteShader(v);
  gl.deleteShader(f);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS) && !gl.isContextLost()) {
    throw new Error(`link: ${gl.getProgramInfoLog(p)}`);
  }
  return p;
}

/** A full-screen triangle (no buffers: positions come from gl_VertexID). */
export const FULLSCREEN_VS = `#version 300 es
out vec2 vUv;
void main() {
  vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
  vUv = p;
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`;

/** Shared GLSL: hashing, value noise, fbm, and a dither against banding. */
export const GLSL_NOISE = `
float hash12(vec2 p) {
  vec3 p3 = fract(vec3(p.xyx) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}
float vnoise(vec2 p) {
  vec2 i = floor(p), f = fract(p);
  vec2 u = f * f * (3.0 - 2.0 * f);
  return mix(mix(hash12(i), hash12(i + vec2(1.0, 0.0)), u.x),
             mix(hash12(i + vec2(0.0, 1.0)), hash12(i + vec2(1.0, 1.0)), u.x), u.y);
}
float fbm(vec2 p) {
  float v = 0.0, a = 0.5;
  mat2 m = mat2(1.6, 1.2, -1.2, 1.6);
  for (int i = 0; i < 5; i++) { v += a * vnoise(p); p = m * p; a *= 0.5; }
  return v;
}
vec3 dither(vec2 frag) {
  return vec3((hash12(frag) - 0.5) / 255.0);
}`;

/** Reads a CSS color variable as linear-ish RGB floats. */
export function cssColor(name: string, fallback: [number, number, number]): [number, number, number] {
  if (typeof document === "undefined") return fallback;
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const m = v.match(/^#([0-9a-f]{6})$/i);
  if (!m?.[1]) return fallback;
  const n = Number.parseInt(m[1], 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}

export function isDark(): boolean {
  return typeof document !== "undefined" && document.documentElement.dataset.theme !== "light";
}

/** Runs a scene on a canvas. Returns a disposer. */
export function mount(canvas: HTMLCanvasElement, scene: Scene, opts: StageOptions = {}): () => void {
  const gl = canvas.getContext("webgl2", {
    alpha: true,
    antialias: false,
    premultipliedAlpha: true,
    powerPreference: "low-power",
  });
  if (!gl) {
    canvas.dataset.webgl = "unavailable";
    return () => {};
  }
  const reduced = prefersReducedMotion() && !opts.alwaysAnimate;
  const scale = opts.scale ?? 0.6;
  const maxDpr = opts.maxDpr ?? 2;
  const minFrame = 1000 / (opts.fps ?? 60);
  let width = 1;
  let height = 1;
  let dpr = 1;
  let raf = 0;
  let start = performance.now();
  let last = start;
  let visible = true;
  let onscreen = true;
  let alive = true;
  let ready = false;

  if (opts.pixel) canvas.style.imageRendering = "pixelated";
  const resize = () => {
    dpr = Math.min(window.devicePixelRatio || 1, maxDpr);
    const r = canvas.getBoundingClientRect();
    const k = opts.pixel ? (window.devicePixelRatio || 1) / opts.pixel : dpr * scale;
    width = Math.max(1, Math.round(r.width * k));
    height = Math.max(1, Math.round(r.height * k));
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
    }
  };

  const init = () => {
    try {
      scene.init(gl);
      ready = true;
    } catch (e) {
      console.warn("fctl: scene init failed", e);
      canvas.dataset.webgl = "failed";
      ready = false;
    }
  };

  const draw = (now: number) => {
    if (!ready || gl.isContextLost()) return;
    const time = reduced ? 12 : (now - start) / 1000;
    const dt = Math.min(0.1, (now - last) / 1000);
    last = now;
    gl.viewport(0, 0, width, height);
    try {
      scene.frame({
        gl,
        width,
        height,
        time,
        dt,
        dpr: opts.pixel ? (window.devicePixelRatio || 1) / opts.pixel : dpr * scale,
      });
    } catch (e) {
      console.warn("fctl: scene frame failed", e);
      ready = false;
    }
  };

  const loop = (now: number) => {
    raf = 0;
    if (!alive || !visible || !onscreen) return;
    if (now - last >= minFrame - 1) draw(now);
    raf = requestAnimationFrame(loop);
  };

  const kick = () => {
    if (!raf && alive && visible && onscreen && !reduced) {
      last = performance.now();
      raf = requestAnimationFrame(loop);
    }
  };

  const ro = new ResizeObserver(() => {
    resize();
    if (reduced) draw(performance.now());
  });
  ro.observe(canvas);
  const io = new IntersectionObserver((entries) => {
    onscreen = entries.some((e) => e.isIntersecting);
    kick();
  });
  io.observe(canvas);
  const onVis = () => {
    visible = document.visibilityState === "visible";
    kick();
  };
  document.addEventListener("visibilitychange", onVis);
  const onLost = (e: Event) => {
    e.preventDefault();
    ready = false;
    cancelAnimationFrame(raf);
    raf = 0;
  };
  const onRestored = () => {
    init();
    kick();
  };
  canvas.addEventListener("webglcontextlost", onLost);
  canvas.addEventListener("webglcontextrestored", onRestored);

  resize();
  init();
  scene.invalidate = () => {
    if (reduced && alive) draw(performance.now());
  };
  if (reduced) draw(performance.now());
  else kick();
  start = performance.now();

  return () => {
    alive = false;
    cancelAnimationFrame(raf);
    ro.disconnect();
    io.disconnect();
    document.removeEventListener("visibilitychange", onVis);
    canvas.removeEventListener("webglcontextlost", onLost);
    canvas.removeEventListener("webglcontextrestored", onRestored);
    if (!gl.isContextLost()) scene.dispose?.(gl);
  };
}
