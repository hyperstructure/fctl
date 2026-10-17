// The landing page: the foundry scene as the hero, with a few imaginary
// sessions at work, plus copy buttons and scroll reveals.

import "./site.css";
import { mount, prefersReducedMotion } from "../../web/src/gl/core";
import { type FoundryLane, foundry, LANES } from "../../web/src/gl/foundry";

document.documentElement.classList.add("js");

function hero() {
  const canvas = document.querySelector<HTMLCanvasElement>(".hero-canvas");
  const section = canvas?.closest<HTMLElement>(".hero");
  if (!canvas || !section) return;
  const scene = foundry({ planet: [1.02, -0.75, 1.15], density: 0.55 });
  mount(canvas, scene.scene, { pixel: 3 });

  // A dozen sessions: mostly working (yellow), some waiting for a reply
  // (blue), now and then one paused (pink). They come and go slowly.
  const lanes = new Map<number, FoundryLane>();
  const pick = () => Math.floor(Math.random() * LANES);
  const color = () => {
    const r = Math.random();
    return r < 0.7 ? 1 : r < 0.92 ? 2 : 3;
  };
  while (lanes.size < 9) lanes.set(pick(), { lit: 1, color: color(), energy: Math.random() * 0.6 });
  scene.setLanes(lanes);
  if (prefersReducedMotion()) return;

  setInterval(() => {
    const working = [...lanes.entries()].filter(([, l]) => l.color === 1);
    const [lane, l] = working[Math.floor(Math.random() * working.length)] ?? [];
    if (lane === undefined || !l) return;
    scene.pulse(lane, 40 + Math.random() * 900);
    l.energy = Math.min(1, l.energy + 0.25);
  }, 420);
  setInterval(() => {
    for (const l of lanes.values()) l.energy *= 0.8;
    // Now and then a session finishes and another starts elsewhere.
    if (Math.random() < 0.18) {
      const keys = [...lanes.keys()];
      const gone = keys[Math.floor(Math.random() * keys.length)];
      if (gone !== undefined) lanes.delete(gone);
      let next = pick();
      while (lanes.has(next)) next = pick();
      lanes.set(next, { lit: 1, color: color(), energy: 0.5 });
    }
    scene.setLanes(lanes);
  }, 1200);

  section.addEventListener("pointermove", (e) => {
    const r = section.getBoundingClientRect();
    scene.pointer((e.clientX - r.left) / r.width, (e.clientY - r.top) / r.height);
  });
  section.addEventListener("pointerleave", () => scene.pointerLeave());
}

function copyButtons() {
  for (const b of document.querySelectorAll<HTMLButtonElement>("button[data-copy]")) {
    b.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(b.dataset.copy ?? "");
        b.textContent = "Copied";
        b.classList.add("done");
        setTimeout(() => {
          b.textContent = "Copy";
          b.classList.remove("done");
        }, 1600);
      } catch {
        b.textContent = "Select and copy";
      }
    });
  }
}

function reveals() {
  const els = document.querySelectorAll(".reveal");
  if (!("IntersectionObserver" in window)) {
    for (const el of els) el.classList.add("seen");
    return;
  }
  const io = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        e.target.classList.add("seen");
        io.unobserve(e.target);
      }
    },
    { rootMargin: "0px 0px -8% 0px", threshold: 0.12 },
  );
  for (const el of els) io.observe(el);
}

hero();
copyButtons();
reveals();
