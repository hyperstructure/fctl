// The agent orb in a session's header: a small sphere of flowing light whose
// color follows the session's state and whose motion follows its activity.

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
void main() {
  vec2 p = (gl_FragCoord.xy - 0.5 * uRes) / (0.5 * min(uRes.x, uRes.y));
  float r = length(p);
  float t = uTime * (0.25 + uEnergy * 1.4);
  vec3 col = vec3(0.0);
  float alpha = 0.0;
  if (r < 1.0) {
    vec3 n = vec3(p, sqrt(1.0 - r * r));
    // Rotate the field over time.
    float c = cos(t * 0.6), s = sin(t * 0.6);
    vec3 q = vec3(c * n.x - s * n.z, n.y, s * n.x + c * n.z);
    float f = noise3(q * 3.2 + vec3(0.0, t * 0.4, 0.0));
    float g = noise3(q * 6.5 - vec3(t * 0.3, 0.0, 0.0));
    float swirl = smoothstep(0.35, 0.85, f * 0.7 + g * 0.45);
    col = mix(uA, uB, swirl);
    float light = 0.45 + 0.55 * pow(max(dot(n, normalize(vec3(-0.4, 0.5, 0.75))), 0.0), 1.5);
    float rim = pow(1.0 - n.z, 2.5);
    col = col * (0.35 + light * (0.55 + uEnergy * 0.5)) + uB * rim * (0.6 + uEnergy);
    col *= mix(1.0, 0.45, uDim);
    alpha = smoothstep(1.0, 0.96, r);
  }
  // Soft halo outside the sphere.
  float halo = exp(-max(r - 1.0, 0.0) * 6.0) * (0.22 + uEnergy * 0.5) * (1.0 - uDim * 0.7);
  col += mix(uA, uB, 0.5) * halo * (1.0 - alpha);
  alpha = max(alpha, halo * 0.9);
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
