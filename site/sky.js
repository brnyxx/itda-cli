// The hero's sky: a field of points drifting through a slow current, with
// threads forming between the ones that come close. A website on the left,
// an agent on the right, and what passes between them: itda. The cursor is
// a hand in the field; points lean toward it and the threads follow.
//
// three.js from cdnjs, drawn at half resolution, paused when the hero is off
// screen or the tab is hidden, one still frame under reduced motion, and
// nothing at all (the CSS glow stays) when WebGL is unavailable.
(() => {
  const hero = document.querySelector(".hero");
  if (!hero || !window.THREE || !window.WebGLRenderingContext) return;
  const T = window.THREE;
  let renderer;
  try {
    renderer = new T.WebGLRenderer({ antialias: false, alpha: true, powerPreference: "low-power" });
  } catch {
    return;
  }
  const canvas = renderer.domElement;
  canvas.className = "sky";
  canvas.setAttribute("aria-hidden", "true");
  hero.prepend(canvas);
  renderer.setClearColor(0x000000, 0);


  // Behind the field: slow liquid chrome in the brand blues, the sky itself.
  const liquid = new T.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    uniforms: { u_time: { value: 0 }, u_mouse: { value: new T.Vector2(0.5, 0.5) }, u_aspect: { value: 1 } },
    vertexShader: `
      varying vec2 v_uv;
      void main() { v_uv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
    fragmentShader: `
      precision mediump float;
      varying vec2 v_uv;
      uniform float u_time;
      uniform vec2 u_mouse;
      uniform float u_aspect;
      float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
      float noise(vec2 p) {
        vec2 i = floor(p), f = fract(p);
        f = f * f * (3.0 - 2.0 * f);
        return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), f.x),
                   mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), f.x), f.y);
      }
      float fbm(vec2 p) {
        float v = 0.0, a = 0.5;
        mat2 r = mat2(0.8, 0.6, -0.6, 0.8);
        for (int i = 0; i < 5; i++) { v += a * noise(p); p = r * p * 2.0 + 10.0; a *= 0.5; }
        return v;
      }
      void main() {
        vec2 uv = v_uv;
        vec2 q = vec2(uv.x * u_aspect, uv.y);
        float t = u_time * 0.045;
        vec2 m = (u_mouse - 0.5) * 0.35;
        vec2 w1 = vec2(fbm(q * 1.6 + t + m), fbm(q * 1.6 - t * 0.7 + 3.1));
        vec2 w2 = vec2(fbm(q * 2.2 + 2.0 * w1 + vec2(1.7, 9.2) + 0.3 * t),
                       fbm(q * 2.2 + 2.0 * w1 + vec2(8.3, 2.8) - 0.2 * t));
        float f = fbm(q * 2.0 + 2.4 * w2);
        float crest = smoothstep(0.52, 0.74, f) * (1.0 - smoothstep(0.74, 0.92, f));
        float sheen = pow(crest, 2.2);
        vec3 ink = vec3(0.043, 0.051, 0.071);
        vec3 blue = vec3(0.184, 0.357, 0.918);
        vec3 lilac = vec3(0.431, 0.545, 1.0);
        vec3 cyan = vec3(0.45, 0.93, 1.0);
        vec3 rose = vec3(1.0, 0.48, 0.71);
        vec3 col = mix(ink, blue * 0.5, smoothstep(0.25, 0.75, f));
        col = mix(col, lilac * 0.85, sheen * 0.9);
        float irid = 0.5 + 0.5 * sin(6.2831 * (w2.x - w2.y) + t * 2.0);
        col += sheen * mix(cyan, rose, irid) * 0.35;
        float shade = smoothstep(0.0, 0.85, length((uv - vec2(0.12, 0.72)) * vec2(1.2, 1.0)));
        col *= mix(0.2, 1.0, shade);
        float line = 0.5 + 0.5 * sin(gl_FragCoord.y * 3.1416);
        col *= 0.95 + 0.05 * line;
        col += (hash(gl_FragCoord.xy + fract(u_time)) - 0.5) * 0.02;
        float alpha = smoothstep(0.0, 0.28, uv.y) * 0.9;
        gl_FragColor = vec4(col * alpha, alpha);
      }`,
  });
  const sky = new T.Mesh(new T.PlaneGeometry(2, 2), liquid);
  sky.frustumCulled = false;
  const skyScene = new T.Scene();
  const skyCamera = new T.OrthographicCamera(-1, 1, 1, -1, 0, 1);
  skyScene.add(sky);

  const scene = new T.Scene();
  scene.fog = new T.FogExp2(0x0b0d12, 0.055);
  const camera = new T.PerspectiveCamera(42, 1, 0.1, 100);
  camera.position.set(0, 0, 16);

  // Points live in a flat slab; two denser clusters stand for the website
  // and the agent, and the current between them carries the rest.
  const N = 640;
  const pos = new Float32Array(N * 3);
  const seed = new Float32Array(N * 3);
  const col = new Float32Array(N * 3);
  const lilac = new T.Color(0x6e8bff), cyan = new T.Color(0x73edff), rose = new T.Color(0xff7ab6), blue = new T.Color(0x2f5bea);
  for (let i = 0; i < N; i++) {
    const side = i % 5 === 0 ? -1 : i % 5 === 1 ? 1 : 0;
    const x = side === 0 ? (Math.random() - 0.5) * 26 : side * (7 + Math.random() * 3) + (Math.random() - 0.5) * 3;
    const y = (Math.random() - 0.5) * (side === 0 ? 12 : 6);
    const z = (Math.random() - 0.5) * 6;
    pos.set([x, y, z], i * 3);
    seed.set([Math.random() * 100, Math.random() * 100, Math.random() * 100], i * 3);
    const c = side === -1 ? cyan : side === 1 ? rose : Math.random() < 0.8 ? lilac : blue;
    col.set([c.r, c.g, c.b], i * 3);
  }
  const points = new T.BufferGeometry();
  points.setAttribute("position", new T.BufferAttribute(pos, 3));
  points.setAttribute("color", new T.BufferAttribute(col, 3));
  const dot = new T.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: T.AdditiveBlending,
    vertexColors: true,
    uniforms: { u_px: { value: 1 } },
    vertexShader: `
      uniform float u_px;
      varying vec3 v_col;
      varying float v_depth;
      void main() {
        v_col = color;
        vec4 mv = modelViewMatrix * vec4(position, 1.0);
        v_depth = -mv.z;
        gl_PointSize = u_px * 64.0 / -mv.z;
        gl_Position = projectionMatrix * mv;
      }`,
    fragmentShader: `
      varying vec3 v_col;
      varying float v_depth;
      void main() {
        float d = length(gl_PointCoord - 0.5);
        float a = smoothstep(0.5, 0.08, d);
        float core = smoothstep(0.18, 0.0, d);
        float fade = clamp(1.0 - (v_depth - 10.0) / 14.0, 0.25, 1.0);
        gl_FragColor = vec4(v_col * (0.55 + core * 0.9), a * 0.85 * fade);
      }`,
  });
  scene.add(new T.Points(points, dot));

  // Threads: a fixed budget of segments, refilled every frame from the
  // pairs that are close enough. Near the cursor the reach grows, so the
  // hand gathers threads around it.
  const MAX = 1800;
  const lpos = new Float32Array(MAX * 6);
  const lcol = new Float32Array(MAX * 6);
  const lines = new T.BufferGeometry();
  lines.setAttribute("position", new T.BufferAttribute(lpos, 3).setUsage(T.DynamicDrawUsage));
  lines.setAttribute("color", new T.BufferAttribute(lcol, 3).setUsage(T.DynamicDrawUsage));
  const thread = new T.LineBasicMaterial({ vertexColors: true, transparent: true, opacity: 0.34, blending: T.AdditiveBlending, depthWrite: false });
  scene.add(new T.LineSegments(lines, thread));

  // One long thread that always exists: the handoff itself, website to agent.
  const spine = new T.BufferGeometry();
  const SP = 160;
  const spos = new Float32Array(SP * 3);
  spine.setAttribute("position", new T.BufferAttribute(spos, 3).setUsage(T.DynamicDrawUsage));
  scene.add(new T.Line(spine, new T.LineBasicMaterial({ color: 0x6e8bff, transparent: true, opacity: 0.5, blending: T.AdditiveBlending, depthWrite: false })));

  const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const mouse = new T.Vector2(0, 0), target = new T.Vector2(0, 0);
  const hand = new T.Vector3(0, 0, 0);
  let visible = true, frame = 0, last = 0;
  const start = performance.now();

  const noise = (x, y, t) =>
    Math.sin(x * 0.41 + t) * Math.cos(y * 0.37 - t * 0.7) + Math.sin((x + y) * 0.23 + t * 0.5) * 0.5;

  const resize = () => {
    const w = hero.clientWidth, h = hero.clientHeight;
    const dpr = Math.min(window.devicePixelRatio || 1, 2) * 0.6;
    renderer.setPixelRatio(dpr);
    renderer.setSize(w, h, false);
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
    dot.uniforms.u_px.value = dpr;
  };

  const step = (now) => {
    const t = still ? 30 : (now - start) / 1000;
    mouse.lerp(target, 0.05);
    // The hand in world space, on the z=0 plane.
    const half = Math.tan((camera.fov * Math.PI) / 360) * camera.position.z;
    hand.set(mouse.x * half * camera.aspect, mouse.y * half, 0);
    camera.position.x = mouse.x * 1.2;
    camera.position.y = mouse.y * 0.8;
    camera.lookAt(0, 0, 0);

    const p = points.attributes.position.array;
    for (let i = 0; i < N; i++) {
      const j = i * 3;
      const sx = seed[j], sy = seed[j + 1];
      // Current: a slow eastward drift folded by noise, wrapped at the edges.
      let x = p[j] + 0.012 + noise(p[j] + sx, p[j + 1] + sy, t * 0.6) * 0.012;
      let y = p[j + 1] + noise(p[j + 1] + sy, p[j] - sx, t * 0.5) * 0.012;
      let z = p[j + 2] + Math.sin(t * 0.4 + sx) * 0.004;
      // The hand pulls what is near it, gently.
      const dx = hand.x - x, dy = hand.y - y;
      const d2 = dx * dx + dy * dy;
      if (d2 < 16) {
        const g = (1 - d2 / 16) * 0.035;
        x += dx * g;
        y += dy * g;
      }
      if (x > 14) x = -14;
      if (y > 7.5) y = -7.5; else if (y < -7.5) y = 7.5;
      if (z > 3.5) z = -3.5; else if (z < -3.5) z = 3.5;
      p[j] = x; p[j + 1] = y; p[j + 2] = z;
    }
    points.attributes.position.needsUpdate = true;

    // Threads between neighbours, nearest-first per point, within budget.
    let n = 0;
    const reach = 1.45;
    for (let i = 0; i < N && n < MAX; i++) {
      const a = i * 3;
      const ax = p[a], ay = p[a + 1], az = p[a + 2];
      const hx = hand.x - ax, hy = hand.y - ay;
      const nearHand = Math.max(0, 1 - (hx * hx + hy * hy) / 20);
      const r = reach + nearHand * 1.1;
      let made = 0;
      for (let k = i + 1; k < N && made < 3 && n < MAX; k++) {
        const b = k * 3;
        const dx = p[b] - ax, dy = p[b + 1] - ay, dz = p[b + 2] - az;
        const d = dx * dx + dy * dy + dz * dz;
        if (d > r * r) continue;
        const w = 1 - Math.sqrt(d) / r;
        const o = n * 6;
        lpos[o] = ax; lpos[o + 1] = ay; lpos[o + 2] = az;
        lpos[o + 3] = p[b]; lpos[o + 4] = p[b + 1]; lpos[o + 5] = p[b + 2];
        const tint = 0.25 + w * 0.75 + nearHand * 0.4;
        for (let c = 0; c < 2; c++) {
          lcol[o + c * 3] = col[a] * tint;
          lcol[o + c * 3 + 1] = col[a + 1] * tint;
          lcol[o + c * 3 + 2] = col[a + 2] * tint;
        }
        n++; made++;
      }
    }
    lines.setDrawRange(0, n * 2);
    lines.attributes.position.needsUpdate = true;
    lines.attributes.color.needsUpdate = true;

    // The spine: a slow wave from the website cluster to the agent cluster.
    for (let i = 0; i < SP; i++) {
      const u = i / (SP - 1);
      const x = -9 + u * 18;
      const y = Math.sin(u * Math.PI * 2 + t * 0.8) * 1.6 * Math.sin(u * Math.PI) + (hand.y * 0.25) * Math.sin(u * Math.PI);
      const z = Math.cos(u * Math.PI * 3 + t * 0.6) * 0.8;
      spos.set([x, y, z], i * 3);
    }
    spine.attributes.position.needsUpdate = true;

    liquid.uniforms.u_time.value = t;
    liquid.uniforms.u_mouse.value.set(mouse.x * 0.5 + 0.5, mouse.y * 0.5 + 0.5);
    liquid.uniforms.u_aspect.value = camera.aspect;
    renderer.autoClear = true;
    renderer.render(skyScene, skyCamera);
    renderer.autoClear = false;
    renderer.render(scene, camera);
  };

  const loop = (now) => {
    frame = 0;
    if (!visible) return;
    if (now - last > 1000 / 45) { step(now); last = now; }
    frame = requestAnimationFrame(loop);
  };

  hero.addEventListener("pointermove", (e) => {
    const r = hero.getBoundingClientRect();
    target.set(((e.clientX - r.left) / r.width) * 2 - 1, -(((e.clientY - r.top) / r.height) * 2 - 1));
  }, { passive: true });
  hero.addEventListener("pointerleave", () => target.set(0, 0), { passive: true });
  window.addEventListener("resize", () => { resize(); if (still) step(performance.now()); }, { passive: true });

  resize();
  if (still) { step(performance.now()); return; }
  new IntersectionObserver((entries) => {
    visible = entries.some((e) => e.isIntersecting);
    if (visible && !frame) frame = requestAnimationFrame(loop);
  }).observe(hero);
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) { cancelAnimationFrame(frame); frame = 0; }
    else if (visible && !frame) frame = requestAnimationFrame(loop);
  });
  frame = requestAnimationFrame(loop);
})();
