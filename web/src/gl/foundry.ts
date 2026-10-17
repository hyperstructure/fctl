// The foundry: fctl's signature scene. A planet's limb rises from the lower
// right; every session is a thread of light streaming from the left into it.
// Working sessions burn yellow, sessions waiting for you glow blue, and
// activity sends a hot pink packet down a thread that flashes where it lands.
//
// The scene renders at a low resolution and is quantized to the brand palette
// with an 8×8 ordered (Bayer) dither, then upscaled without smoothing: every
// pixel is one exact brand color or nothing. One fullscreen pass, ~40 lane
// evaluations per low-res pixel.

import { compile, cssColor, FULLSCREEN_VS, GLSL_NOISE, isDark, type Scene } from "./core";

export const LANES = 40;

const FS = `#version 300 es
precision highp float;
out vec4 outColor;
uniform vec2 uRes;
uniform float uTime;
uniform float uDark;
uniform vec4 uLane[${LANES}];   // x: lit (0..1), y: color index, z: energy, w: phase
uniform vec2 uPulse[${LANES}];  // x: start time, y: weight (0 = none)
uniform vec3 uPlanet;           // x, y: center, z: radius (all in heights)
uniform vec2 uEnd[${LANES}];    // where each thread meets the limb (heights)
uniform vec3 uMouse;            // xy: pointer (0..1, y up), z: strength
uniform float uDensity;         // how many threads show at all (0..1)
uniform vec3 uCol[5];           // thread colors: neutral, working, waiting, paused, failed
uniform vec3 uInk;              // stars, rim (white on dark, ink on light)
uniform vec3 uHot;              // packets (hot pink)
uniform vec3 uFlash;            // landing flashes (yellow)
uniform vec3 uBodyNear;         // planet body near the limb
uniform vec3 uBodyDeep;         // planet body further in
uniform vec3 uAir;              // the glow around the planet
${GLSL_NOISE}

float bayer8(vec2 p) {
  ivec2 q = ivec2(mod(p, 8.0));
  int a = q.x ^ q.y;
  int y = q.y;
  int v = ((a & 1) << 5) | ((y & 1) << 4) | ((a & 2) << 2) | ((y & 2) << 1) | ((a & 4) >> 1) | ((y & 4) >> 2);
  return (float(v) + 0.5) / 64.0;
}

void main() {
  vec2 px = floor(gl_FragCoord.xy);
  float aspect = uRes.x / uRes.y;
  float pixel = 1.0 / uRes.y;
  vec2 p = gl_FragCoord.xy / uRes.y;
  vec2 C = uPlanet.xy;
  float R = uPlanet.z;
  vec2 rel = p - C;
  float sd = length(rel) - R;
  float th = bayer8(px);

  // Threads.
  float best = 0.0;
  vec3 bestCol = vec3(0.0);
  float flash = 0.0;
  for (int i = 0; i < ${LANES}; i++) {
    float lane = float(i);
    float k = lane / ${LANES - 1}.0;
    vec4 L = uLane[i];
    float lit = L.x;
    // Sparse when quiet: only some idle threads show.
    float shown = max(lit, step(fract(lane * 0.618 + 0.11), uDensity));
    if (shown <= 0.0) continue;
    vec2 E = uEnd[i];
    if (p.x > E.x + 3.0 * pixel) continue;
    float s = clamp(p.x / max(E.x, 0.001), 0.0, 1.0);
    float e = s * s * (3.0 - 2.0 * s);
    float y0 = mix(0.05, 0.95, k);
    float y = mix(y0, E.y, pow(e, 1.35));
    float amp = (0.03 + 0.02 * lit) * (1.0 - e);
    y += sin(s * 6.2831 * (0.6 + fract(lane * 0.618) * 0.6) + uTime * 0.12 + lane * 1.7) * amp;
    y += sin(s * 6.2831 * (1.5 + fract(lane * 0.381)) - uTime * 0.08 + lane) * amp * 0.35;
    vec2 dm = vec2((s * E.x / aspect - uMouse.x) * aspect, y - uMouse.y);
    y += sign(dm.y) * exp(-dot(dm, dm) / 0.02) * uMouse.z * 0.045;
    float d = abs(p.y - y);
    if (d > 0.06) continue;

    float core = exp(-pow(d / (pixel * mix(0.75, 1.15, lit)), 2.0));
    float halo = exp(-d / (pixel * 6.0)) * 0.2 * lit;
    float speed = 0.035 + 0.08 * lit + 0.14 * L.z;
    float flow = 0.3 + 0.7 * vnoise(vec2(s * 16.0 - uTime * speed * 16.0, lane * 7.31));
    float c = fract(s * 2.4 - uTime * speed * 1.6 + L.w);
    float spark = pow(c, 26.0) * smoothstep(1.0, 0.97, c) * lit;
    float boost = 0.0;
    vec2 P = uPulse[i];
    if (P.y > 0.0) {
      float head = (uTime - P.x) * 0.45;
      float dd = s - head;
      boost = P.y * (dd < 0.0 ? exp(dd * 9.0) : exp(-dd * 150.0)) * step(head, 1.02);
      // Where the packet lands, the limb flashes.
      float since = (uTime - P.x) - 1.0 / 0.45;
      if (since > 0.0 && since < 1.2) {
        float r = length(p - E);
        float ring = exp(-pow((r - since * 0.06) / (pixel * 2.0), 2.0)) * (1.0 - since / 1.2);
        flash = max(flash, ring * P.y);
      }
    }
    float fadeIn = smoothstep(0.0, 0.1, s);
    float base = mix(0.14, 0.92, lit) * flow;
    float inten = (core * (base + spark * 1.2 + boost * 1.6) + halo * (flow + boost * 2.5)) * fadeIn;
    if (inten > best) {
      best = inten;
      vec3 col = mix(uCol[0], uCol[int(L.y + 0.5)], lit);
      if (boost * core > 0.3 || boost > 0.6) col = uHot;
      else if (spark * core > 0.55 && uDark > 0.5) col = vec3(1.0);
      bestCol = col;
    }
  }

  // The planet: a dithered body that darkens inward, a hairline limb, a glow.
  float body = 0.0;
  vec3 bodyCol = uBodyNear;
  if (sd < 0.0) {
    float depth = -sd / R;
    float band = vnoise(vec2(atan(rel.y, rel.x) * 22.0, depth * 70.0 - uTime * 0.05));
    body = exp(-depth * 11.0) * (0.7 + 0.3 * band);
    bodyCol = mix(uBodyNear, uBodyDeep, smoothstep(0.0, 0.07, depth));
  }
  float limb = exp(-pow(sd / (pixel * 1.05), 2.0));
  float air = sd > 0.0 ? exp(-sd * 24.0) * 0.32 : 0.0;

  // Stars, with dust gathering near the limb.
  float h = hash12(px * 0.913 + 17.0);
  float dust = sd > 0.0 ? 0.05 * exp(-sd * 7.0) : 0.0;
  float twinkle = 0.55 + 0.45 * sin(uTime * (0.6 + h * 2.4) + h * 40.0);
  float star = h > 1.0 - (0.0022 + dust) ? twinkle : 0.0;

  // Film grain on the empty dark.
  float g = hash12(px + floor(uTime * 10.0) * 13.17);

  vec3 col = vec3(0.0);
  float a = 0.0;
  if (flash > th) {
    col = uFlash; a = 1.0;
  } else if (best > th) {
    col = bestCol; a = 1.0;
  } else if (limb > th) {
    col = uInk; a = 1.0;
  } else if (star > 0.5) {
    col = uInk; a = star;
  } else if (body > th) {
    col = bodyCol; a = 1.0;
  } else if (air > th) {
    col = uAir; a = 1.0;
  } else if (g > 0.94) {
    col = uInk; a = uDark > 0.5 ? 0.05 : 0.035;
  }
  outColor = vec4(col * a, a);
}`;

export interface FoundryLane {
  /** 0 idle .. 1 fully lit. */
  lit: number;
  /** Palette index: 0 neutral, 1 working, 2 waiting, 3 paused, 4 failed. */
  color: number;
  /** Recent activity, 0..1 (speeds the flow up). */
  energy: number;
}

export interface FoundryOptions {
  /** Planet center x (fraction of the width), center y and radius (in heights, for a 16:10 view). */
  planet?: [number, number, number];
  /** Fraction of idle threads drawn (0..1). */
  density?: number;
}

export interface FoundryHandle {
  scene: Scene;
  setLanes(lanes: Map<number, FoundryLane>): void;
  pulse(lane: number, weight: number): void;
  /** Pointer in 0..1 (y down, like the DOM). */
  pointer(x: number, y: number): void;
  pointerLeave(): void;
}

export function foundry(opts: FoundryOptions = {}): FoundryHandle {
  let prog: WebGLProgram | null = null;
  let vao: WebGLVertexArrayObject | null = null;
  const u: Record<string, WebGLUniformLocation | null> = {};
  const lanes = new Float32Array(LANES * 4);
  const target = new Float32Array(LANES * 4);
  const pulses = new Float32Array(LANES * 2);
  const ends = new Float32Array(LANES * 2);
  const mouse = new Float32Array([0.5, 0.5, 0]);
  const mouseTarget = new Float32Array([0.5, 0.5, 0]);
  const planet = opts.planet ?? [0.92, -0.62, 1.2];
  const density = opts.density ?? 0.55;
  let now = 0;
  let first = true;
  for (let i = 0; i < LANES; i++) {
    lanes[i * 4 + 3] = Math.random();
    target[i * 4 + 3] = lanes[i * 4 + 3] ?? 0;
  }
  const names = [
    "uRes",
    "uTime",
    "uDark",
    "uLane",
    "uPulse",
    "uPlanet",
    "uEnd",
    "uMouse",
    "uDensity",
    "uCol",
    "uInk",
    "uHot",
    "uFlash",
    "uBodyNear",
    "uBodyDeep",
    "uAir",
  ];

  const scene: Scene = {
    init(gl) {
      prog = compile(gl, FULLSCREEN_VS, FS);
      vao = gl.createVertexArray();
      for (const n of names) u[n] = gl.getUniformLocation(prog, n);
    },
    frame({ gl, width, height, time, dt }) {
      if (!prog) return;
      now = time;
      const k = 1 - Math.exp(-dt * 3);
      for (let i = 0; i < LANES; i++) {
        const o = i * 4;
        lanes[o] = (lanes[o] ?? 0) + ((target[o] ?? 0) - (lanes[o] ?? 0)) * k;
        // The color changes only while the thread is dark or lit, never mid-fade.
        if ((lanes[o] ?? 0) < 0.05 || (target[o] ?? 0) > 0) lanes[o + 1] = target[o + 1] ?? 0;
        lanes[o + 2] = (lanes[o + 2] ?? 0) + ((target[o + 2] ?? 0) - (lanes[o + 2] ?? 0)) * k;
      }
      const km = 1 - Math.exp(-dt * 6);
      for (let i = 0; i < 3; i++) mouse[i] = (mouse[i] ?? 0) + ((mouseTarget[i] ?? 0) - (mouse[i] ?? 0)) * km;

      const dark = isDark();
      gl.disable(gl.BLEND);
      gl.useProgram(prog);
      gl.bindVertexArray(vao);
      gl.uniform2f(u.uRes ?? null, width, height);
      gl.uniform1f(u.uTime ?? null, time);
      gl.uniform1f(u.uDark ?? null, dark ? 1 : 0);
      gl.uniform4fv(u.uLane ?? null, lanes);
      gl.uniform2fv(u.uPulse ?? null, pulses);
      // The planet shrinks on narrow screens so the threads keep room.
      const aspect = width / Math.max(1, height);
      const fit = Math.min(1, Math.max(0.6, aspect / 1.6));
      const cx = planet[0] * aspect;
      const cy = planet[1] * fit;
      const r = planet[2] * fit;
      gl.uniform3f(u.uPlanet ?? null, cx, cy, r);
      // Threads land along the visible upper-left arc of the limb.
      const bottom = Math.PI - Math.asin(Math.min(0.99, Math.max(-0.99, -cy / r)));
      for (let i = 0; i < LANES; i++) {
        const a = bottom - 0.05 + (1.68 - (bottom - 0.05)) * (i / (LANES - 1));
        ends[i * 2] = cx + r * Math.cos(a);
        ends[i * 2 + 1] = cy + r * Math.sin(a);
      }
      gl.uniform2fv(u.uEnd ?? null, ends);
      gl.uniform3fv(u.uMouse ?? null, mouse);
      gl.uniform1f(u.uDensity ?? null, density);
      const ink = dark ? cssColor("--c-ink", [0.98, 0.98, 0.98]) : cssColor("--c-ink", [0.09, 0.09, 0.09]);
      const pal = [
        dark ? cssColor("--c-ink-3", [0.55, 0.55, 0.55]) : cssColor("--c-ink-4", [0.64, 0.64, 0.64]),
        dark ? cssColor("--c-accent", [1, 1, 0.36]) : ink,
        cssColor("--c-blue", [0, 0.49, 1]),
        cssColor("--c-pink", [1, 0, 0.85]),
        cssColor("--c-danger", [1, 0, 0.85]),
      ];
      gl.uniform3fv(u.uCol ?? null, new Float32Array(pal.flat()));
      gl.uniform3fv(u.uInk ?? null, ink);
      gl.uniform3fv(u.uHot ?? null, cssColor("--c-pink", [1, 0, 0.85]));
      gl.uniform3fv(u.uFlash ?? null, dark ? cssColor("--c-accent", [1, 1, 0.36]) : ink);
      gl.uniform3fv(u.uBodyNear ?? null, cssColor("--c-pink", [1, 0, 0.85]));
      gl.uniform3fv(u.uBodyDeep ?? null, cssColor("--c-blue", [0, 0.49, 1]));
      gl.uniform3fv(u.uAir ?? null, cssColor("--c-blue", [0, 0.49, 1]));
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      gl.bindVertexArray(null);
    },
    dispose(gl) {
      if (prog) gl.deleteProgram(prog);
      if (vao) gl.deleteVertexArray(vao);
    },
  };

  return {
    scene,
    setLanes(map) {
      for (let i = 0; i < LANES; i++) {
        const l = map.get(i);
        target[i * 4] = l ? l.lit : 0;
        if (l) target[i * 4 + 1] = l.color;
        target[i * 4 + 2] = l ? l.energy : 0;
      }
      // The first lanes appear at once; later changes ease in.
      if (first) {
        first = false;
        for (let i = 0; i < LANES * 4; i++) if (i % 4 !== 3) lanes[i] = target[i] ?? 0;
      }
      scene.invalidate?.();
    },
    pulse(lane, weight) {
      if (lane < 0 || lane >= LANES) return;
      pulses[lane * 2] = now;
      pulses[lane * 2 + 1] = Math.min(1, 0.35 + Math.log10(1 + weight) / 3);
    },
    pointer(x, y) {
      mouseTarget[0] = x;
      mouseTarget[1] = 1 - y;
      mouseTarget[2] = 1;
    },
    pointerLeave() {
      mouseTarget[2] = 0;
    },
  };
}
