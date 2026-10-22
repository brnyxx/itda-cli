// The sky behind the whole page: a living network. Nodes drift on a slow
// current and spring back toward home, threads form between the ones that
// come close, and packets of light run from node to node along whatever
// threads exist this frame. A website hub on the left, an agent hub on the
// right, and the handoff between them: itda.
//
// The cursor is a node in the graph. Nodes near it gather and orbit, its
// motion leaves a wake, threads run from it to its neighbours, and a click
// sends a ring through the field that the springs knit back together.
//
// Under it all: liquid chrome with striped reflections and a wire grid to
// the horizon. three.js from cdnjs, drawn at reduced resolution, paused when
// the tab is hidden, one still frame under reduced motion, and nothing at
// all (the CSS glow stays) when WebGL is unavailable.
(() => {
  if (!window.THREE || !window.WebGLRenderingContext) return;
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
  document.body.prepend(canvas);
  renderer.setClearColor(0x000000, 0);

  const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const coarse = window.matchMedia("(pointer: coarse)").matches;

  // Liquid chrome, kept dim so the graph in front of it reads first.
  const liquid = new T.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    uniforms: {
      u_time: { value: 0 },
      u_mouse: { value: new T.Vector2(0.5, 0.5) },
      u_aspect: { value: 1 },
      u_scroll: { value: 0 },
    },
    vertexShader: `
      varying vec2 v_uv;
      void main() { v_uv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
    fragmentShader: `
      precision mediump float;
      varying vec2 v_uv;
      uniform float u_time;
      uniform vec2 u_mouse;
      uniform float u_aspect;
      uniform float u_scroll;
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
      float stripe(float f, float t) { return pow(0.5 + 0.5 * sin(f * 26.0 - t * 1.8), 7.0); }
      vec3 holo(float h) { return 0.5 + 0.5 * cos(6.2831 * (h + vec3(0.0, 0.33, 0.67))); }
      void main() {
        vec2 uv = v_uv;
        vec2 q = vec2(uv.x * u_aspect, uv.y + u_scroll);
        float t = u_time * 0.045;
        vec2 m = (u_mouse - 0.5) * 0.35;
        vec2 w1 = vec2(fbm(q * 1.4 + t + m), fbm(q * 1.4 - t * 0.7 + 3.1));
        vec2 w2 = vec2(fbm(q * 2.0 + 2.0 * w1 + vec2(1.7, 9.2) + 0.3 * t),
                       fbm(q * 2.0 + 2.0 * w1 + vec2(8.3, 2.8) - 0.2 * t));
        float f = fbm(q * 1.8 + 2.4 * w2);
        vec3 ink = vec3(0.043, 0.051, 0.071);
        vec3 steel = vec3(0.14, 0.17, 0.24);
        vec3 blue = vec3(0.184, 0.357, 0.918);
        vec3 col = mix(ink, steel, smoothstep(0.3, 0.8, f));
        col = mix(col, blue * 0.5, smoothstep(0.55, 0.85, f) * 0.6);
        float sr = stripe(f + 0.006, t), sg = stripe(f, t), sb = stripe(f - 0.006, t);
        col += vec3(0.86, 0.9, 1.0) * vec3(sr, sg, sb) * 0.3;
        float h = w2.x - w2.y + t * 0.3;
        col += holo(h) * sg * 0.26;
        col += holo(h + 0.5) * smoothstep(0.62, 0.78, f) * 0.07;
        float shade = smoothstep(0.0, 0.9, length((uv - vec2(0.1, 0.7)) * vec2(1.25, 1.0)));
        col *= mix(0.22, 0.66, shade);
        float line = 0.5 + 0.5 * sin(gl_FragCoord.y * 3.1416);
        col *= 0.88 + 0.12 * line;
        col += (hash(gl_FragCoord.xy + fract(u_time)) - 0.5) * 0.03;
        gl_FragColor = vec4(col * 0.92, 0.92);
      }`,
  });
  const skyScene = new T.Scene();
  const skyCamera = new T.OrthographicCamera(-1, 1, 1, -1, 0, 1);
  const skyMesh = new T.Mesh(new T.PlaneGeometry(2, 2), liquid);
  skyMesh.frustumCulled = false;
  skyScene.add(skyMesh);

  const scene = new T.Scene();
  scene.fog = new T.FogExp2(0x0b0d12, 0.045);
  const camera = new T.PerspectiveCamera(42, 1, 0.1, 120);
  camera.position.set(0, 0, 16);

  // The floor: a wire grid to the horizon, cyan near and magenta far.
  const GRID = 32, GW = 60, GD = 70;
  const gpos = new Float32Array((GRID + 1) * 2 * 2 * 3);
  const gcol = new Float32Array((GRID + 1) * 2 * 2 * 3);
  const gNear = new T.Color(0x73edff), gFar = new T.Color(0xff5fd0);
  for (let i = 0, g = 0; i <= GRID; i++) {
    const u = i / GRID;
    const x = -GW / 2 + u * GW, z = -GD + u * GD;
    gpos.set([x, 0, -GD, x, 0, 0], g * 6);
    gcol.set([gFar.r, gFar.g, gFar.b, gNear.r, gNear.g, gNear.b], g * 6);
    g++;
    const c = gFar.clone().lerp(gNear, u);
    gpos.set([-GW / 2, 0, z, GW / 2, 0, z], g * 6);
    gcol.set([c.r, c.g, c.b, c.r, c.g, c.b], g * 6);
    g++;
  }
  const grid = new T.BufferGeometry();
  grid.setAttribute("position", new T.BufferAttribute(gpos, 3));
  grid.setAttribute("color", new T.BufferAttribute(gcol, 3));
  const floor = new T.LineSegments(grid, new T.LineBasicMaterial({ vertexColors: true, transparent: true, opacity: 0.3, blending: T.AdditiveBlending, depthWrite: false }));
  floor.position.set(0, -8.5, 6);
  scene.add(floor);

  // Nodes. The count follows the viewport, with a lower cap on touch
  // screens. The first two are the hubs: the website and the agent.
  const area = window.innerWidth * window.innerHeight;
  const N = Math.max(240, Math.min(coarse ? 360 : 900, Math.round(area / 1500)));
  const pos = new Float32Array(N * 3);
  const vel = new Float32Array(N * 3);
  const home = new Float32Array(N * 3);
  const seed = new Float32Array(N * 2);
  const col = new Float32Array(N * 3);
  const size = new Float32Array(N);
  const lilac = new T.Color(0x9fb0ff), cyan = new T.Color(0x73edff), rose = new T.Color(0xff7ab6), blue = new T.Color(0x5b82ff);
  const XW = 15, YW = 8.5;
  for (let i = 0; i < N; i++) {
    const hub = i < 2;
    const side = hub ? (i === 0 ? -1 : 1) : i % 5 === 0 ? -1 : i % 5 === 1 ? 1 : 0;
    const x = hub ? side * 8 : side === 0 ? (Math.random() - 0.5) * 2 * XW : side * (6.5 + Math.random() * 3.5) + (Math.random() - 0.5) * 3;
    const y = hub ? 0 : (Math.random() - 0.5) * 2 * (side === 0 ? YW : 4.5);
    const z = hub ? 0 : (Math.random() - 0.5) * 5;
    pos.set([x, y, z], i * 3);
    home.set([x, y, z], i * 3);
    seed.set([Math.random() * 100, Math.random() * 100], i * 2);
    const c = hub ? (side < 0 ? cyan : rose) : side === -1 ? cyan : side === 1 ? rose : Math.random() < 0.75 ? lilac : blue;
    col.set([c.r, c.g, c.b], i * 3);
    size[i] = hub ? 3.4 : Math.random() < 0.08 ? 1.9 : 0.7 + Math.random() * 0.7;
  }
  const nodes = new T.BufferGeometry();
  nodes.setAttribute("position", new T.BufferAttribute(pos, 3).setUsage(T.DynamicDrawUsage));
  nodes.setAttribute("color", new T.BufferAttribute(col, 3));
  nodes.setAttribute("size", new T.BufferAttribute(size, 1));
  const dot = new T.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: T.AdditiveBlending,
    vertexColors: true,
    uniforms: { u_px: { value: 1 }, u_hand: { value: new T.Vector3(99, 99, 0) } },
    vertexShader: `
      uniform float u_px;
      uniform vec3 u_hand;
      attribute float size;
      varying vec3 v_col;
      varying float v_depth;
      varying float v_near;
      void main() {
        v_col = color;
        float d = distance(position.xy, u_hand.xy);
        v_near = smoothstep(6.0, 0.0, d);
        vec4 mv = modelViewMatrix * vec4(position, 1.0);
        v_depth = -mv.z;
        gl_PointSize = u_px * size * (1.0 + v_near * 1.4) * 58.0 / -mv.z;
        gl_Position = projectionMatrix * mv;
      }`,
    fragmentShader: `
      varying vec3 v_col;
      varying float v_depth;
      varying float v_near;
      void main() {
        float d = length(gl_PointCoord - 0.5);
        float a = smoothstep(0.5, 0.1, d);
        float core = smoothstep(0.2, 0.0, d);
        float fade = clamp(1.0 - (v_depth - 10.0) / 14.0, 0.3, 1.0);
        vec3 c = mix(v_col, vec3(1.0), v_near * 0.5) * (0.6 + core * 1.1);
        gl_FragColor = vec4(c, a * fade * (0.9 + v_near * 0.1));
      }`,
  });
  scene.add(new T.Points(nodes, dot));

  // Threads, refilled every frame from the pairs that are close enough;
  // the hand's own threads come first in the buffer.
  const MAX = Math.round(N * 4.5);
  const lpos = new Float32Array(MAX * 6);
  const lcol = new Float32Array(MAX * 6);
  const lines = new T.BufferGeometry();
  lines.setAttribute("position", new T.BufferAttribute(lpos, 3).setUsage(T.DynamicDrawUsage));
  lines.setAttribute("color", new T.BufferAttribute(lcol, 3).setUsage(T.DynamicDrawUsage));
  scene.add(new T.LineSegments(lines, new T.LineBasicMaterial({ vertexColors: true, transparent: true, opacity: 0.55, blending: T.AdditiveBlending, depthWrite: false })));

  // The spine: the handoff itself, website to agent, always there.
  const SP = 160;
  const spos = new Float32Array(SP * 3);
  const spine = new T.BufferGeometry();
  spine.setAttribute("position", new T.BufferAttribute(spos, 3).setUsage(T.DynamicDrawUsage));
  scene.add(new T.Line(spine, new T.LineBasicMaterial({ color: 0x8fa6ff, transparent: true, opacity: 0.55, blending: T.AdditiveBlending, depthWrite: false })));

  // Packets: light running node to node along live threads, plus a few that
  // ride the spine from the website to the agent and back.
  const P = Math.round(N / 7);
  const pk = [];
  for (let i = 0; i < P; i++) {
    pk.push({ from: 2 + Math.floor(Math.random() * (N - 2)), to: -1, u: Math.random(), speed: 0.6 + Math.random() * 0.8, spine: i < 6, dir: i % 2 ? 1 : -1 });
  }
  const ppos = new Float32Array(P * 3);
  const packets = new T.BufferGeometry();
  packets.setAttribute("position", new T.BufferAttribute(ppos, 3).setUsage(T.DynamicDrawUsage));
  const spark = new T.ShaderMaterial({
    transparent: true, depthWrite: false, blending: T.AdditiveBlending,
    uniforms: { u_px: { value: 1 } },
    vertexShader: `
      uniform float u_px;
      void main() {
        vec4 mv = modelViewMatrix * vec4(position, 1.0);
        gl_PointSize = u_px * 38.0 / -mv.z;
        gl_Position = projectionMatrix * mv;
      }`,
    fragmentShader: `
      void main() {
        float d = length(gl_PointCoord - 0.5);
        float a = smoothstep(0.5, 0.05, d);
        float core = smoothstep(0.22, 0.0, d);
        gl_FragColor = vec4(mix(vec3(0.45, 0.93, 1.0), vec3(1.0), core), a);
      }`,
  });
  scene.add(new T.Points(packets, spark));

  // Spatial hash over x and y; z is shallow enough to ignore for lookups.
  const CELL = 3.4;
  const CX = Math.ceil((2 * XW + 8) / CELL), CY = Math.ceil((2 * YW + 8) / CELL);
  const cellHead = new Int32Array(CX * CY);
  const cellNext = new Int32Array(N);
  const cellX = (x) => Math.min(CX - 1, Math.max(0, Math.floor((x + XW + 4) / CELL)));
  const cellY = (y) => Math.min(CY - 1, Math.max(0, Math.floor((y + YW + 4) / CELL)));
  const found = new Int32Array(64);
  // Nodes within r of (x, y), skipping `self`, at most 64.
  const around = (x, y, r, self) => {
    let n = 0;
    const r2 = r * r;
    const cx = cellX(x), cy = cellY(y);
    for (let oy = -1; oy <= 1; oy++) {
      const yy = cy + oy;
      if (yy < 0 || yy >= CY) continue;
      for (let ox = -1; ox <= 1; ox++) {
        const xx = cx + ox;
        if (xx < 0 || xx >= CX) continue;
        for (let k = cellHead[yy * CX + xx]; k !== -1 && n < 64; k = cellNext[k]) {
          if (k === self) continue;
          const dx = pos[k * 3] - x, dy = pos[k * 3 + 1] - y;
          if (dx * dx + dy * dy <= r2) found[n++] = k;
        }
      }
    }
    return n;
  };

  const mouse = new T.Vector2(0, 0), target = new T.Vector2(0, 0);
  const hand = new T.Vector3(0, 0, 0), handPrev = new T.Vector3(0, 0, 0), handVel = new T.Vector2(0, 0);
  const away = new T.Vector3(99, 99, 0), click = new T.Vector3();
  let handOn = false;
  const ripples = [];
  let frame = 0, last = 0, scroll = 0, tPrev = 0;
  const start = performance.now();

  const noise = (x, y, t) =>
    Math.sin(x * 0.41 + t) * Math.cos(y * 0.37 - t * 0.7) + Math.sin((x + y) * 0.23 + t * 0.5) * 0.5;

  const toWorld = (nx, ny, out) => {
    const half = Math.tan((camera.fov * Math.PI) / 360) * camera.position.z;
    out.set(nx * half * camera.aspect + camera.position.x, ny * half + camera.position.y, 0);
  };

  const resize = () => {
    const w = window.innerWidth, h = window.innerHeight;
    const dpr = Math.min(window.devicePixelRatio || 1, 2) * (coarse ? 0.45 : 0.6);
    renderer.setPixelRatio(dpr);
    renderer.setSize(w, h, false);
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
    dot.uniforms.u_px.value = dpr;
    spark.uniforms.u_px.value = dpr;
  };

  const step = (now) => {
    const t = still ? 30 : (now - start) / 1000;
    const dt = still ? 1 : Math.min(2, (t - tPrev) * 60 || 1);
    tPrev = t;
    mouse.lerp(target, 0.12);
    scroll += ((window.scrollY || 0) - scroll) * 0.08;
    const s = Math.min(scroll / 1400, 1);
    camera.position.x = mouse.x * 1.0;
    camera.position.y = mouse.y * 0.6 - s * 2.5;
    camera.lookAt(0, -s * 1.5, 0);

    handPrev.copy(hand);
    toWorld(mouse.x, mouse.y, hand);
    handVel.set(hand.x - handPrev.x, hand.y - handPrev.y);
    dot.uniforms.u_hand.value.copy(handOn ? hand : away);

    // Move the nodes.
    const R = 6.5;
    for (let i = 0; i < N; i++) {
      const j = i * 3;
      const hub = i < 2;
      let x = pos[j], y = pos[j + 1], z = pos[j + 2];
      let vx = vel[j], vy = vel[j + 1], vz = vel[j + 2];
      const sx = seed[i * 2], sy = seed[i * 2 + 1];
      // Spring home, stronger for hubs.
      const k = hub ? 0.03 : 0.0035;
      vx += (home[j] - x) * k;
      vy += (home[j + 1] - y) * k;
      vz += (home[j + 2] - z) * k * 2;
      if (!hub) {
        // Current.
        vx += 0.006 + noise(x + sx, y + sy, t * 0.6) * 0.009;
        vy += noise(y + sy, x - sx, t * 0.5) * 0.009;
        vz += Math.sin(t * 0.4 + sx) * 0.002;
        // The hand: gather, orbit, and the wake of its motion.
        if (handOn) {
          const dx = hand.x - x, dy = hand.y - y;
          const d = Math.sqrt(dx * dx + dy * dy);
          if (d < R && d > 0.001) {
            const w = 1 - d / R;
            const g = w * w * 0.11;
            vx += (dx / d) * g + (-dy / d) * w * 0.035 + handVel.x * w * 0.35;
            vy += (dy / d) * g + (dx / d) * w * 0.035 + handVel.y * w * 0.35;
            if (d < 1.2) { vx -= (dx / d) * (1.2 - d) * 0.08; vy -= (dy / d) * (1.2 - d) * 0.08; }
          }
        }
      }
      // Ripples: a ring that pushes outward as it passes.
      for (const rp of ripples) {
        const age = t - rp.t0;
        const rr = age * 11;
        const dx = x - rp.x, dy = y - rp.y;
        const d = Math.sqrt(dx * dx + dy * dy) || 0.001;
        const band = 1 - Math.min(1, Math.abs(d - rr) / 1.6);
        if (band > 0) {
          const f = band * (1 - age / 1.3) * 0.28;
          vx += (dx / d) * f;
          vy += (dy / d) * f;
        }
      }
      vx *= 0.9; vy *= 0.9; vz *= 0.9;
      x += vx * dt; y += vy * dt; z += vz * dt;
      if (x > XW + 2) { x = -XW - 2; home[j] = x; } else if (x < -XW - 2) { x = XW + 2; home[j] = x; }
      if (y > YW + 2) y = YW + 2; else if (y < -YW - 2) y = -YW - 2;
      if (z > 3) z = 3; else if (z < -3) z = -3;
      pos[j] = x; pos[j + 1] = y; pos[j + 2] = z;
      vel[j] = vx; vel[j + 1] = vy; vel[j + 2] = vz;
    }
    for (let i = ripples.length - 1; i >= 0; i--) if (t - ripples[i].t0 > 1.3) ripples.splice(i, 1);
    nodes.attributes.position.needsUpdate = true;

    // Rebuild the hash.
    cellHead.fill(-1);
    for (let i = 0; i < N; i++) {
      const c = cellY(pos[i * 3 + 1]) * CX + cellX(pos[i * 3]);
      cellNext[i] = cellHead[c];
      cellHead[c] = i;
    }

    // Threads: from the hand first, then between neighbours.
    let n = 0;
    const put = (ax, ay, az, bx, by, bz, r, g, b, r2, g2, b2) => {
      const o = n * 6;
      lpos[o] = ax; lpos[o + 1] = ay; lpos[o + 2] = az;
      lpos[o + 3] = bx; lpos[o + 4] = by; lpos[o + 5] = bz;
      lcol[o] = r; lcol[o + 1] = g; lcol[o + 2] = b;
      lcol[o + 3] = r2; lcol[o + 4] = g2; lcol[o + 5] = b2;
      n++;
    };
    if (handOn) {
      const m = around(hand.x, hand.y, 4.2, -1);
      for (let q = 0; q < m && n < MAX; q++) {
        const b = found[q] * 3;
        const dx = pos[b] - hand.x, dy = pos[b + 1] - hand.y;
        const w = 0.4 + (1 - Math.sqrt(dx * dx + dy * dy) / 4.2);
        put(hand.x, hand.y, 0, pos[b], pos[b + 1], pos[b + 2], 1, 1, 1, col[b] * w, col[b + 1] * w, col[b + 2] * w);
      }
    }
    const reach = 1.7;
    for (let i = 0; i < N && n < MAX; i++) {
      const a = i * 3;
      const ax = pos[a], ay = pos[a + 1], az = pos[a + 2];
      let nearHand = 0;
      if (handOn) {
        const hx = hand.x - ax, hy = hand.y - ay;
        nearHand = Math.max(0, 1 - Math.sqrt(hx * hx + hy * hy) / R);
      }
      const hub = i < 2;
      const r = (hub ? 3.2 : reach) + nearHand * 1.5;
      const m = around(ax, ay, r, i);
      let made = 0;
      for (let q = 0; q < m && made < (hub ? 12 : 4) && n < MAX; q++) {
        const k = found[q];
        if (k < i && k >= 2) continue; // each pair once; hubs may link anyone
        const b = k * 3;
        const dx = pos[b] - ax, dy = pos[b + 1] - ay;
        const w = 1 - Math.sqrt(dx * dx + dy * dy) / r;
        const tint = 0.2 + w * 0.8 + nearHand * 0.6;
        put(ax, ay, az, pos[b], pos[b + 1], pos[b + 2],
          col[a] * tint, col[a + 1] * tint, col[a + 2] * tint,
          col[b] * tint, col[b + 1] * tint, col[b + 2] * tint);
        made++;
      }
    }
    lines.setDrawRange(0, n * 2);
    lines.attributes.position.needsUpdate = true;
    lines.attributes.color.needsUpdate = true;

    // The spine, website hub to agent hub.
    const hx0 = pos[0], hy0 = pos[1], hx1 = pos[3], hy1 = pos[4];
    for (let i = 0; i < SP; i++) {
      const u = i / (SP - 1);
      const bend = Math.sin(u * Math.PI);
      const x = hx0 + (hx1 - hx0) * u;
      let y = hy0 + (hy1 - hy0) * u + Math.sin(u * Math.PI * 2 + t * 0.8) * 1.4 * bend;
      if (handOn) y += (hand.y - y) * 0.12 * bend;
      const z = Math.cos(u * Math.PI * 3 + t * 0.6) * 0.8;
      spos.set([x, y, z], i * 3);
    }
    spine.attributes.position.needsUpdate = true;

    // Packets.
    for (let i = 0; i < P; i++) {
      const p = pk[i];
      if (p.spine) {
        if (!still) p.u += p.dir * p.speed * 0.004 * dt;
        if (p.u > 1 || p.u < 0) { p.u = p.u > 1 ? 1 : 0; p.dir = -p.dir; }
        const k = Math.round(p.u * (SP - 1)) * 3;
        ppos[i * 3] = spos[k]; ppos[i * 3 + 1] = spos[k + 1]; ppos[i * 3 + 2] = spos[k + 2];
        continue;
      }
      if (p.to < 0 || p.u >= 1) {
        if (p.to >= 0) p.from = p.to;
        p.u = 0;
        const m = around(pos[p.from * 3], pos[p.from * 3 + 1], 2.6, p.from);
        p.to = m ? found[Math.floor(Math.random() * m)] : 2 + Math.floor(Math.random() * (N - 2));
      }
      const a = p.from * 3, b = p.to * 3;
      let boost = 1;
      if (handOn) {
        const dx = hand.x - pos[a], dy = hand.y - pos[a + 1];
        boost += Math.max(0, 1 - Math.sqrt(dx * dx + dy * dy) / R) * 2.5;
      }
      if (!still) p.u += p.speed * boost * 0.02 * dt;
      const u = Math.min(1, p.u);
      const e = u * u * (3 - 2 * u);
      ppos[i * 3] = pos[a] + (pos[b] - pos[a]) * e;
      ppos[i * 3 + 1] = pos[a + 1] + (pos[b + 1] - pos[a + 1]) * e;
      ppos[i * 3 + 2] = pos[a + 2] + (pos[b + 2] - pos[a + 2]) * e;
    }
    packets.attributes.position.needsUpdate = true;

    const cell = GD / GRID;
    floor.position.z = 6 + ((t * 0.9 + scroll * 0.01) % cell);

    liquid.uniforms.u_time.value = t;
    liquid.uniforms.u_mouse.value.set(mouse.x * 0.5 + 0.5, mouse.y * 0.5 + 0.5);
    liquid.uniforms.u_aspect.value = camera.aspect;
    liquid.uniforms.u_scroll.value = scroll * 0.0004;
    renderer.autoClear = true;
    renderer.render(skyScene, skyCamera);
    renderer.autoClear = false;
    renderer.render(scene, camera);
  };

  const loop = (now) => {
    frame = 0;
    if (document.hidden) return;
    if (now - last > 1000 / 45) { step(now); last = now; }
    frame = requestAnimationFrame(loop);
  };

  const point = (e) => {
    target.set((e.clientX / window.innerWidth) * 2 - 1, -((e.clientY / window.innerHeight) * 2 - 1));
    handOn = true;
  };
  window.addEventListener("pointermove", point, { passive: true });
  window.addEventListener("pointerdown", (e) => {
    point(e);
    toWorld(target.x, target.y, click);
    ripples.push({ x: click.x, y: click.y, t0: (performance.now() - start) / 1000 });
    if (ripples.length > 6) ripples.shift();
  }, { passive: true });
  document.documentElement.addEventListener("mouseleave", () => { handOn = false; });
  // On touch screens the hand is only there while a finger is down.
  if (coarse) {
    const lift = () => { handOn = false; };
    window.addEventListener("pointerup", lift, { passive: true });
    window.addEventListener("pointercancel", lift, { passive: true });
  }
  window.addEventListener("resize", () => { resize(); if (still) step(performance.now()); }, { passive: true });

  resize();
  if (still) { step(performance.now()); return; }
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) { cancelAnimationFrame(frame); frame = 0; }
    else if (!frame) frame = requestAnimationFrame(loop);
  });
  frame = requestAnimationFrame(loop);
})();
