// The agent orb in a session's header: a small dithered planet whose colors
// follow the session's state and whose motion follows its activity.

import { compile, FULLSCREEN_VS, GLSL_NOISE, isDark, type Scene } from "./core";

const FS = `#version 300 es
precision highp float;
in vec2 vUv;
out vec4 outColor;
uniform vec2 uRes;
uniform float uTime;
uniform vec3 uA;
uniform vec3 uB;
uniform float uEnergy;
uniform float uDim;
${GLSL_NOISE}
float noise3(vec3 p) {
  // Two offset 2D noises make a cheap 3D-ish field over the sphere.
  return vnoise(p.xy + p.z * 1.7) * 0.5 + vnoise(p.yz * 1.3 - p.x * 0.9) * 0.5;
}
float bayer8(vec2 p) {
  ivec2 q = ivec2(mod(p, 8.0));
  int a = q.x ^ q.y;
  int y = q.y;
  int v = ((a & 1) << 5) | ((y & 1) << 4) | ((a & 2) << 2) | ((y & 2) << 1) | ((a & 4) >> 1) | ((y & 4) >> 2);
  return (float(v) + 0.5) / 64.0;
}
void main() {
  // A tiny dithered planet, like the foundry's: two brand colors, no blending.
  vec2 px = floor(gl_FragCoord.xy);
  vec2 p = (gl_FragCoord.xy - 0.5 * uRes) / (0.5 * min(uRes.x, uRes.y)) * 1.16;
  float r = length(p);
  float th = bayer8(px);
  float th2 = bayer8(px + vec2(3.0, 5.0));
  float t = uTime * (0.25 + uEnergy * 1.4);
  vec3 col = vec3(0.0);
  float alpha = 0.0;
  if (r < 1.0) {
    vec3 n = vec3(p, sqrt(1.0 - r * r));
    float c = cos(t * 0.6), s = sin(t * 0.6);
    vec3 q = vec3(c * n.x - s * n.z, n.y, s * n.x + c * n.z);
    float f = noise3(q * 3.2 + vec3(0.0, t * 0.4, 0.0));
    float g = noise3(q * 6.5 - vec3(t * 0.3, 0.0, 0.0));
    float swirl = smoothstep(0.3, 0.85, f * 0.7 + g * 0.45);
    float light = pow(max(dot(n, normalize(vec3(-0.45, 0.55, 0.7))), 0.0), 1.1);
    float rim = pow(1.0 - n.z, 3.0);
    float b = (0.18 + 0.82 * light) * (0.62 + uEnergy * 0.38) * mix(1.0, 0.45, uDim) + rim * 0.35;
    if (b > th) {
      col = swirl > th2 ? uB : uA;
      alpha = 1.0;
    }
  } else {
    float halo = exp(-(r - 1.0) * 9.0) * (0.18 + uEnergy * 0.55) * (1.0 - uDim * 0.8);
    if (halo > th) {
      col = uB;
      alpha = 1.0;
    }
  }
  outColor = vec4(col * alpha, alpha);
}`;

export interface OrbState {
  a: [number, number, number];
  b: [number, number, number];
  energy: number;
  dim: number;
}

export function orb(get: () => OrbState): Scene {
  let prog: WebGLProgram | null = null;
  let vao: WebGLVertexArrayObject | null = null;
  const u: Record<string, WebGLUniformLocation | null> = {};
  let energy = 0;
  return {
    init(gl) {
      prog = compile(gl, FULLSCREEN_VS, FS);
      vao = gl.createVertexArray();
      for (const n of ["uRes", "uTime", "uA", "uB", "uEnergy", "uDim"]) u[n] = gl.getUniformLocation(prog, n);
    },
    frame({ gl, width, height, time, dt }) {
      if (!prog) return;
      const s = get();
      energy += (s.energy - energy) * (1 - Math.exp(-dt * 2.5));
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.enable(gl.BLEND);
      gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
      gl.useProgram(prog);
      gl.bindVertexArray(vao);
      gl.uniform2f(u.uRes ?? null, width, height);
      gl.uniform1f(u.uTime ?? null, time);
      gl.uniform3fv(u.uA ?? null, s.a);
      gl.uniform3fv(u.uB ?? null, s.b);
      gl.uniform1f(u.uEnergy ?? null, energy);
      gl.uniform1f(u.uDim ?? null, s.dim * (isDark() ? 1 : 0.8));
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      gl.disable(gl.BLEND);
    },
    dispose(gl) {
      if (prog) gl.deleteProgram(prog);
      if (vao) gl.deleteVertexArray(vao);
    },
  };
}
