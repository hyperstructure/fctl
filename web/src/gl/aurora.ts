// Behind the sign-in pages: slow, domain-warped folds of light.

import { compile, cssColor, FULLSCREEN_VS, GLSL_NOISE, isDark, type Scene } from "./core";

const FS = `#version 300 es
precision highp float;
in vec2 vUv;
out vec4 outColor;
uniform vec2 uRes;
uniform float uTime;
uniform vec3 uA;
uniform vec3 uB;
uniform vec3 uC;
uniform vec3 uBase;
uniform float uDark;
${GLSL_NOISE}
void main() {
  vec2 p = (gl_FragCoord.xy - 0.5 * uRes) / uRes.y;
  float t = uTime * 0.028;
  // Domain warping (two levels): soft folds that drift.
  vec2 q = vec2(fbm(p * 1.15 + vec2(0.0, t)), fbm(p * 1.15 + vec2(5.2, -t * 0.7)));
  vec2 r = vec2(fbm(p * 1.4 + 2.2 * q + vec2(1.7, 9.2) + t * 0.55),
                fbm(p * 1.4 + 2.2 * q + vec2(8.3, 2.8) - t * 0.4));
  float f = fbm(p * 1.1 + 2.6 * r);
  // Light gathers along the ridges of the field.
  float ridge = pow(smoothstep(0.35, 0.95, f), 2.2);
  vec3 col = mix(uA, uB, clamp(length(r) * 0.9 - 0.15, 0.0, 1.0));
  col = mix(col, uC, smoothstep(0.55, 1.0, q.y) * 0.55);
  float vignette = smoothstep(1.25, 0.15, length(p * vec2(0.9, 1.2)));
  float strength = mix(0.55, 0.9, uDark) * vignette;
  vec3 light = col * ridge * strength + col * pow(f, 6.0) * 0.6 * strength;
  vec3 c = uDark > 0.5 ? uBase + light : uBase - (vec3(1.0) - col) * ridge * 0.16 * vignette;
  outColor = vec4(c + dither(gl_FragCoord.xy), 1.0);
}`;

export function aurora(): Scene {
  let prog: WebGLProgram | null = null;
  let vao: WebGLVertexArrayObject | null = null;
  const u: Record<string, WebGLUniformLocation | null> = {};
  return {
    init(gl) {
      prog = compile(gl, FULLSCREEN_VS, FS);
      vao = gl.createVertexArray();
      for (const n of ["uRes", "uTime", "uA", "uB", "uC", "uBase", "uDark"])
        u[n] = gl.getUniformLocation(prog, n);
    },
    frame({ gl, width, height, time }) {
      if (!prog) return;
      const dark = isDark();
      gl.useProgram(prog);
      gl.bindVertexArray(vao);
      gl.uniform2f(u.uRes ?? null, width, height);
      gl.uniform1f(u.uTime ?? null, time);
      gl.uniform3fv(u.uA ?? null, cssColor("--c-accent", [0.31, 0.88, 0.79]));
      gl.uniform3fv(u.uB ?? null, cssColor("--c-blue", [0.52, 0.68, 1.0]));
      gl.uniform3fv(u.uC ?? null, cssColor("--c-violet", [0.73, 0.64, 1.0]));
      gl.uniform3fv(
        u.uBase ?? null,
        cssColor("--c-canvas", dark ? [0.03, 0.035, 0.045] : [0.965, 0.96, 0.945]),
      );
      gl.uniform1f(u.uDark ?? null, dark ? 1 : 0);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
    },
    dispose(gl) {
      if (prog) gl.deleteProgram(prog);
      if (vao) gl.deleteVertexArray(vao);
    },
  };
}
