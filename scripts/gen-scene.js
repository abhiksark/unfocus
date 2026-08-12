// scripts/gen-scene.js — generates the Unfocus Ridgeline scene as crafted SVG.
// Smooth noise-based ridge silhouettes with asymmetric envelopes, baked once.
// Usage: bun gen-scene.js > scene.svg

const W = 1600;
const H = 900;

function mulberry32(seed) {
  return function () {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// Smooth 1D value noise: random values on an integer grid, cosine-interpolated.
function valueNoise(rand, cells) {
  const g = Array.from({ length: cells + 1 }, () => rand());
  return (x) => {
    const t = Math.min(Math.max(x, 0), 1) * cells;
    const i = Math.floor(t);
    const f = t - i;
    const s = (1 - Math.cos(f * Math.PI)) / 2;
    return g[i] * (1 - s) + g[Math.min(i + 1, cells)] * s;
  };
}

// Asymmetric mountain envelope: steep on one side, long shoulder on the other.
function envelope(x, cx, width, skew) {
  const d = x - cx;
  const w = d < 0 ? width : width * skew;
  return Math.exp(-(d * d) / (2 * w * w));
}

// Build one ridge from a list of envelopes + noise, Catmull-Rom smoothed.
function ridge({ seed, base, amp, peaks, broadW, detail }) {
  const rand = mulberry32(seed);
  const broad = valueNoise(rand, 5);
  const fine = valueNoise(rand, 17);
  const n = 76;
  const pts = [];
  for (let i = 0; i <= n; i++) {
    const x = -0.02 + (i / n) * 1.04; // 2% bleed each side for drift
    let shape = broad(x) * broadW + fine(x) * detail;
    for (const p of peaks) shape += envelope(x, p.cx, p.w, p.skew) * p.a;
    const y = base - amp * shape;
    pts.push([x * W, y]);
  }
  let d = `M${(-0.02 * W).toFixed(0)} ${H} L${pts[0][0].toFixed(1)} ${pts[0][1].toFixed(1)}`;
  for (let i = 0; i < pts.length - 1; i++) {
    const p0 = pts[Math.max(i - 1, 0)];
    const p1 = pts[i];
    const p2 = pts[i + 1];
    const p3 = pts[Math.min(i + 2, pts.length - 1)];
    const c1 = [p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6];
    const c2 = [p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6];
    d += ` C${c1[0].toFixed(1)} ${c1[1].toFixed(1)}, ${c2[0].toFixed(1)} ${c2[1].toFixed(1)}, ${p2[0].toFixed(1)} ${p2[1].toFixed(1)}`;
  }
  d += ` L${(1.02 * W).toFixed(0)} ${H} Z`;
  return d;
}

function stars(seed, count) {
  const rand = mulberry32(seed);
  // Separate stream for twinkle timing so adding it left every star where the
  // accepted composition placed it.
  const timing = mulberry32(seed + 1);
  let out = "";
  for (let i = 0; i < count; i++) {
    const x = (rand() * W).toFixed(0);
    const y = (rand() * 320 + 40).toFixed(0);
    const r = (0.8 + rand() * 1.0).toFixed(1);
    const o = (0.14 + rand() * 0.3).toFixed(2);
    const dur = (10 + timing() * 8).toFixed(1);
    const delay = (-timing() * 18).toFixed(1);
    // The opacity attribute is the resting value when animations are off;
    // the custom properties drive the stepped twinkle in BreakOverlay.svelte.
    out += `    <circle class="star" cx="${x}" cy="${y}" r="${r}" fill="#d7ecdc" opacity="${o}" style="--o:${o};--tw-dur:${dur}s;--tw-delay:${delay}s"/>\n`;
  }
  return out;
}

// ---- composition ----
// Horizon low; one defined summit left-of-centre (x≈0.32) is the far focal;
// the centre stays low so the break copy and timer sit against calm sky.
const layers = [
  {
    seed: 11, base: 655, amp: 250, broadW: 0.16, detail: 0.1,
    peaks: [{ cx: 0.32, w: 0.085, skew: 1.35, a: 0.9 }],
    color: "#23493b",
  },
  {
    seed: 47, base: 682, amp: 120, broadW: 0.22, detail: 0.1,
    peaks: [{ cx: 0.78, w: 0.13, skew: 1.6, a: 0.75 }],
    color: "#16352a",
  },
  {
    seed: 23, base: 726, amp: 80, broadW: 0.26, detail: 0.09,
    peaks: [{ cx: 0.1, w: 0.2, skew: 2.2, a: 0.5 }],
    color: "#0c1f1a",
  },
  {
    seed: 5, base: 800, amp: 66, broadW: 0.3, detail: 0.05,
    peaks: [{ cx: 0.6, w: 0.34, skew: 1.4, a: 0.45 }],
    color: "#060e0b",
  },
];

const ridgePaths = layers
  .map(
    (l, i) =>
      `    <path class="ridge ridge-${i + 1}" d="${ridge(l)}" fill="${l.color}"/>`
  )
  .join("\n");

const svg = `<svg class="scene" viewBox="0 0 ${W} ${H}" preserveAspectRatio="xMidYMid slice" aria-hidden="true">
  <defs>
    <linearGradient id="sky" x1="0" y1="0" x2="0" y2="${H}" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#060f0d"/>
      <stop offset="0.6" stop-color="#0b1c17"/>
      <stop offset="1" stop-color="#102a22"/>
    </linearGradient>
    <radialGradient id="haze" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="#9fd8b4" stop-opacity="0.15"/>
      <stop offset="0.55" stop-color="#9fd8b4" stop-opacity="0.05"/>
      <stop offset="1" stop-color="#9fd8b4" stop-opacity="0"/>
    </radialGradient>
    <radialGradient id="mistv" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="#9fd8b4" stop-opacity="0.5"/>
      <stop offset="0.7" stop-color="#9fd8b4" stop-opacity="0.18"/>
      <stop offset="1" stop-color="#9fd8b4" stop-opacity="0"/>
    </radialGradient>
    <radialGradient id="mistamber" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="#e4c088" stop-opacity="0.5"/>
      <stop offset="0.7" stop-color="#e4c088" stop-opacity="0.18"/>
      <stop offset="1" stop-color="#e4c088" stop-opacity="0"/>
    </radialGradient>
    <radialGradient id="dawn" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="#e8c489" stop-opacity="0.5"/>
      <stop offset="0.45" stop-color="#dcb87a" stop-opacity="0.2"/>
      <stop offset="1" stop-color="#d9bb7d" stop-opacity="0"/>
    </radialGradient>
    <mask id="crescent">
      <circle cx="1178" cy="162" r="30" fill="#fff"/>
      <circle cx="1167" cy="154" r="27" fill="#000"/>
    </mask>
  </defs>

  <rect width="${W}" height="${H}" fill="url(#sky)"/>
  <g class="stars">
${stars(99, 24)}  </g>
  <circle class="moonbody" cx="1178" cy="162" r="30" fill="#d7ecdc" opacity="0.85" mask="url(#crescent)"/>

  <!-- moonlit haze gathers behind the far summit; swaps warm at dawn -->
  <ellipse class="haze" cx="512" cy="600" rx="520" ry="270" fill="url(#haze)"/>
  <ellipse class="dawn" cx="512" cy="655" rx="700" ry="280" fill="url(#dawn)"/>

${ridgePaths}

  <!-- mist pools sit in the valleys, not across the frame -->
  <ellipse class="mist" cx="920" cy="660" rx="360" ry="52" fill="url(#mistv)" opacity="0.11"/>
  <ellipse class="mist" cx="210" cy="708" rx="380" ry="54" fill="url(#mistv)" opacity="0.1"/>
  <ellipse class="mist-amber" cx="920" cy="660" rx="360" ry="52" fill="url(#mistamber)" opacity="0"/>
  <ellipse class="mist-amber" cx="210" cy="708" rx="380" ry="54" fill="url(#mistamber)" opacity="0"/>
</svg>`;

console.log(svg);
