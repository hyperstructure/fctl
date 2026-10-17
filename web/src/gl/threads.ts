// The factory floor: a loom of fine threads flowing left to right, gathering
// toward the right edge. Every active session lights one thread in its status
// color; activity sends a bright packet of light along it. Threads part
// gently around the pointer.
//
// Each thread is an instanced ribbon (a triangle strip a few pixels wide), so
// only pixels near a thread are shaded: cheap enough to run at full device
// resolution on any GPU.

import { compile, cssColor, GLSL_NOISE, isDark, type Scene } from "./core";

export const LANES = 40;
const SEGMENTS = 220;
const MAX_PULSES = 32;

const VS = `#version 300 es
precision highp float;
layout(location = 0) in vec2 aPos; // x: along the thread (0..1), y: side (-1 or 1)
uniform vec2 uRes;
uniform float uTime;
uniform float uDpr;
uniform vec4 uLane[${LANES}];   // x: lit (0..1), y: color index, z: energy, w: phase
uniform vec3 uMouse;            // xy: pointer (0..1, y up), z: strength
uniform vec3 uCol[5];
out float vSide;
out float vHw;
out float vU;
out vec3 vColor;
flat out int vLane;

float curve(float lane, float x, vec4 L) {
  float y0 = mix(0.08, 0.92, lane / ${LANES - 1}.0);
  // Gather toward the right edge, where the work comes together.
  float g = smoothstep(0.18, 1.08, x);
  float y = mix(y0, 0.56 + (y0 - 0.5) * 0.34, g * g);
  float amp = (0.040 + 0.018 * L.x) * (1.0 - 0.7 * g);
  float f1 = 0.55 + fract(lane * 0.618) * 0.5;
  float f2 = 1.3 + fract(lane * 0.381) * 0.8;
  y += sin(x * 6.2831 * f1 + uTime * 0.11 + lane * 1.7) * amp;
  y += sin(x * 6.2831 * f2 - uTime * 0.07 + lane * 0.9) * amp * 0.35;
  // Part around the pointer.
  vec2 d = vec2((x - uMouse.x) * uRes.x / uRes.y, y - uMouse.y);
  y += sign(d.y) * exp(-dot(d, d) / 0.018) * uMouse.z * 0.045;
  return y;
}

void main() {
  float lane = float(gl_InstanceID);
  vec4 L = uLane[gl_InstanceID];
  float x = mix(-0.02, 1.02, aPos.x);
  vec2 p = vec2(x, curve(lane, x, L)) * uRes;
  vec2 q = vec2(x + 0.003, curve(lane, x + 0.003, L)) * uRes;
  vec2 dir = normalize(q - p);
  vec2 n = vec2(-dir.y, dir.x);
  float hw = mix(3.5, 24.0, L.x) * uDpr;
  vSide = aPos.y * hw;
  vHw = hw;
  vU = x;
  vLane = gl_InstanceID;
  vColor = mix(uCol[0], uCol[int(L.y + 0.5)], L.x);
  vec2 px = p + n * aPos.y * hw;
  gl_Position = vec4(px / uRes * 2.0 - 1.0, 0.0, 1.0);
}`;

const FS = `#version 300 es
precision highp float;
in float vSide;
in float vHw;
in float vU;
in vec3 vColor;
flat in int vLane;
uniform float uTime;
uniform float uDpr;
uniform float uDark;
uniform vec4 uLane[${LANES}];
uniform vec4 uPulse[${MAX_PULSES}]; // x: lane, y: start time, z: weight
out vec4 outColor;
${GLSL_NOISE}

void main() {
  vec4 L = uLane[vLane];
  float lit = L.x;
  float lane = float(vLane);
  float d = abs(vSide);
  float cw = mix(0.5, 0.85, lit) * uDpr;
  float core = exp(-(d * d) / (cw * cw));
  float halo = exp(-d / (vHw * 0.26)) * smoothstep(vHw, vHw * 0.5, d);

  // Idle threads shimmer slowly; lit ones carry packets along their length.
  float speed = 0.035 + 0.09 * lit + 0.14 * L.z;
  float flow = 0.45 + 0.55 * vnoise(vec2(vU * 11.0 - uTime * speed * 11.0, lane * 7.31));
  float c = fract(vU * 2.6 - uTime * speed * 1.8 + L.w);
  // A comet: bright head, long tail, and a soft leading edge (no seam at the wrap).
  float spark = pow(c, 28.0) * smoothstep(1.0, 0.975, c) * lit;
  float boost = 0.0;
  for (int i = 0; i < ${MAX_PULSES}; i++) {
    vec4 p = uPulse[i];
    if (p.z <= 0.0 || abs(p.x - lane) > 0.5) continue;
    float head = (uTime - p.y) * 0.5 - 0.04;
    float dd = vU - head;
    boost += p.z * (dd < 0.0 ? exp(dd * 12.0) : exp(-dd * 160.0)) * step(head, 1.15);
  }

  float edge = smoothstep(0.0, 0.12, vU) * smoothstep(1.02, 0.84, vU);
  float idle = uDark > 0.5 ? 0.20 : 0.30;
  float coreA = mix(idle, 0.9, lit) * flow + spark * 1.3 + boost * 1.8;
  float haloA = (0.09 * lit * flow + spark * 0.22 + boost * 0.5) * mix(0.45, 1.0, uDark);
  float a = min(1.0, (core * coreA + halo * haloA) * edge);
  // Packets run white-hot at their core.
  vec3 col = mix(vColor, vec3(1.0), clamp((spark + boost) * core * 0.5, 0.0, 0.6) * uDark);
  outColor = vec4(col * a, a);
}`;

export interface ThreadLane {
  /** 0 idle .. 1 fully lit. */
  lit: number;
  /** Index into the palette: 1 accent, 2 blue, 3 amber, 4 danger. */
  color: number;
  /** Recent activity, 0..1 (speeds the flow up). */
  energy: number;
}

export interface ThreadsHandle {
  scene: Scene;
  setLanes(lanes: Map<number, ThreadLane>): void;
  pulse(lane: number, weight: number): void;
  /** Pointer position in 0..1 (y down, like the DOM), or null when it leaves. */
  pointer(x: number, y: number): void;
  pointerLeave(): void;
}

export function threads(): ThreadsHandle {
  let prog: WebGLProgram | null = null;
  let vao: WebGLVertexArrayObject | null = null;
  let buf: WebGLBuffer | null = null;
  const u: Record<string, WebGLUniformLocation | null> = {};
  const lanes = new Float32Array(LANES * 4);
  const target = new Float32Array(LANES * 4);
  const pulses = new Float32Array(MAX_PULSES * 4);
  const mouse = new Float32Array([0.5, 0.5, 0]);
  const mouseTarget = new Float32Array([0.5, 0.5, 0]);
  let pulseNext = 0;
  let now = 0;
  for (let i = 0; i < LANES; i++) {
    lanes[i * 4 + 1] = 1;
    lanes[i * 4 + 3] = Math.random();
    target[i * 4 + 1] = 1;
    target[i * 4 + 3] = lanes[i * 4 + 3] ?? 0;
  }

  const scene: Scene = {
    init(gl) {
      prog = compile(gl, VS, FS);
      for (const n of ["uRes", "uTime", "uLane", "uPulse", "uDpr", "uCol", "uMouse", "uDark"])
        u[n] = gl.getUniformLocation(prog, n);
      const data = new Float32Array((SEGMENTS + 1) * 4);
      for (let i = 0; i <= SEGMENTS; i++) {
        data.set([i / SEGMENTS, -1, i / SEGMENTS, 1], i * 4);
      }
      vao = gl.createVertexArray();
      gl.bindVertexArray(vao);
      buf = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, buf);
      gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
      gl.enableVertexAttribArray(0);
      gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
      gl.bindVertexArray(null);
    },
    frame({ gl, width, height, time, dt, dpr }) {
      if (!prog) return;
      now = time;
      const k = 1 - Math.exp(-dt * 3);
      for (let i = 0; i < LANES; i++) {
        const o = i * 4;
        lanes[o] = (lanes[o] ?? 0) + ((target[o] ?? 0) - (lanes[o] ?? 0)) * k;
        // Color snaps only while the thread is dark, so it never flashes.
        if ((lanes[o] ?? 0) < 0.05 || (target[o] ?? 0) > 0) lanes[o + 1] = target[o + 1] ?? 1;
        lanes[o + 2] = (lanes[o + 2] ?? 0) + ((target[o + 2] ?? 0) - (lanes[o + 2] ?? 0)) * k;
      }
      const km = 1 - Math.exp(-dt * 6);
      for (let i = 0; i < 3; i++) mouse[i] = (mouse[i] ?? 0) + ((mouseTarget[i] ?? 0) - (mouse[i] ?? 0)) * km;

      const dark = isDark();
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.enable(gl.BLEND);
      // Light adds up on dark backgrounds; ink layers on light ones.
      if (dark) gl.blendFunc(gl.ONE, gl.ONE);
      else gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
      gl.useProgram(prog);
      gl.bindVertexArray(vao);
      gl.uniform2f(u.uRes ?? null, width, height);
      gl.uniform1f(u.uTime ?? null, time);
      gl.uniform1f(u.uDpr ?? null, dpr);
      gl.uniform1f(u.uDark ?? null, dark ? 1 : 0);
      gl.uniform4fv(u.uLane ?? null, lanes);
      gl.uniform4fv(u.uPulse ?? null, pulses);
      gl.uniform3fv(u.uMouse ?? null, mouse);
      const idle = dark
        ? cssColor("--c-ink-3", [0.55, 0.57, 0.6])
        : cssColor("--c-ink-3", [0.42, 0.44, 0.47]);
      const pal = [
        idle,
        cssColor("--c-accent", [0.31, 0.88, 0.79]),
        cssColor("--c-blue", [0.52, 0.68, 1]),
        cssColor("--c-amber", [0.96, 0.72, 0.38]),
        cssColor("--c-danger", [1, 0.51, 0.47]),
      ];
      gl.uniform3fv(u.uCol ?? null, new Float32Array(pal.flat()));
      gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, (SEGMENTS + 1) * 2, LANES);
      gl.bindVertexArray(null);
      gl.disable(gl.BLEND);
    },
    dispose(gl) {
      if (prog) gl.deleteProgram(prog);
      if (vao) gl.deleteVertexArray(vao);
      if (buf) gl.deleteBuffer(buf);
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
    },
    pulse(lane, weight) {
      const o = (pulseNext++ % MAX_PULSES) * 4;
      pulses[o] = lane;
      pulses[o + 1] = now;
      pulses[o + 2] = Math.min(1, 0.3 + Math.log10(1 + weight) / 3);
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
