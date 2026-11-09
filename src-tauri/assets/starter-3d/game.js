// Sky Islands: a complete three.js starter. Pick a hero, double-jump across
// floating islands, collect every gem. three.js is vendored in ./vendor/three,
// heroes live in ./assets/characters (any .glb listed in characters.json).
import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { RoomEnvironment } from 'three/addons/environments/RoomEnvironment.js';

const $ = id => document.getElementById(id);
const canvas = $('game');

/* ---------------------------------------------------------------- renderer */
const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, powerPreference: 'high-performance' });
renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
renderer.outputColorSpace = THREE.SRGBColorSpace;
renderer.toneMapping = THREE.ACESFilmicToneMapping;
renderer.toneMappingExposure = 0.95;
renderer.shadowMap.enabled = true;
renderer.shadowMap.type = THREE.PCFSoftShadowMap;

const scene = new THREE.Scene();
const pmrem = new THREE.PMREMGenerator(renderer);
scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
scene.environmentIntensity = 0.3;
scene.fog = new THREE.Fog(0xc8dcf5, 90, 300);

const camera = new THREE.PerspectiveCamera(58, 1, 0.1, 900);

/* --------------------------------------------------------------------- sky */
const sky = new THREE.Mesh(
  new THREE.SphereGeometry(500, 32, 16),
  new THREE.ShaderMaterial({
    side: THREE.BackSide, depthWrite: false, fog: false,
    uniforms: {
      top: { value: new THREE.Color(0x2f62d4) },
      horizon: { value: new THREE.Color(0xffd9b8) },
      bottom: { value: new THREE.Color(0x86b3e8) },
    },
    vertexShader: 'varying vec3 vDir; void main(){ vDir = normalize(position); gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }',
    fragmentShader: `uniform vec3 top; uniform vec3 horizon; uniform vec3 bottom; varying vec3 vDir;
      void main(){
        float h = vDir.y;
        vec3 c = h > 0.0 ? mix(horizon, top, pow(h, 0.32)) : mix(horizon, bottom, pow(-h, 0.45));
        gl_FragColor = vec4(c, 1.0);
        #include <tonemapping_fragment>
        #include <colorspace_fragment>
      }`,
  }));
scene.add(sky);

/* ------------------------------------------------------------------ lights */
scene.add(new THREE.HemisphereLight(0xcfe0ff, 0x4a3b2c, 0.55));
const sun = new THREE.DirectionalLight(0xfff0d6, 2.6);
sun.castShadow = true;
sun.shadow.mapSize.set(2048, 2048);
sun.shadow.bias = -0.0004;
sun.shadow.normalBias = 0.03;
Object.assign(sun.shadow.camera, { left: -22, right: 22, top: 22, bottom: -22, near: 1, far: 80 });
scene.add(sun, sun.target);

/* ------------------------------------------------------------------- world */
// A small seeded random, so the world is the same every time.
let seed = 7;
const rand = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);

// Each island: centre, radius and the height of its grass.
const islands = [
  { x: 0, z: 0, r: 13, top: 0 },
  { x: -11, z: -14, r: 4.5, top: 1.6 },
  { x: 7, z: -21, r: 5, top: 2.4 },
  { x: -7, z: -33, r: 4.4, top: 4.0 },
  { x: 10, z: -38, r: 3.8, top: 3.2 },
  { x: 2, z: -47, r: 5.2, top: 5.6 },
  { x: -12, z: -52, r: 4.2, top: 7.0 },
  { x: 0, z: -66, r: 8, top: 8.2 },
];
const grassMat = new THREE.MeshStandardMaterial({ color: 0x4f9e3c, roughness: 0.95 });
const dirtMat = new THREE.MeshStandardMaterial({ color: 0x8a6542, roughness: 1, flatShading: true });
const rockMat = new THREE.MeshStandardMaterial({ color: 0x7d7f86, roughness: 0.9, flatShading: true });
for (const s of islands) {
  const top = new THREE.Mesh(new THREE.CylinderGeometry(s.r, s.r * 0.97, 0.7, 36), grassMat);
  top.position.set(s.x, s.top - 0.35, s.z);
  top.receiveShadow = top.castShadow = true;
  const underGeo = new THREE.ConeGeometry(s.r * 0.96, s.r * 1.5, 9, 3);
  const p = underGeo.attributes.position;
  for (let i = 0; i < p.count; i++) {
    if (p.getY(i) < s.r * 0.7) p.setXYZ(i, p.getX(i) * (0.85 + rand() * 0.3), p.getY(i), p.getZ(i) * (0.85 + rand() * 0.3));
  }
  underGeo.computeVertexNormals();
  const under = new THREE.Mesh(underGeo, dirtMat);
  under.rotation.x = Math.PI;
  under.position.set(s.x, s.top - 0.7 - s.r * 0.75, s.z);
  under.castShadow = true;
  scene.add(top, under);
}

// Trees and rocks, instanced: one draw call each, however many there are.
const spots = [];
for (const s of islands) {
  const n = Math.round(s.r * 0.9);
  for (let i = 0; i < n; i++) {
    const a = rand() * Math.PI * 2, d = s.r * (0.55 + rand() * 0.35);
    spots.push({ x: s.x + Math.cos(a) * d, y: s.top, z: s.z + Math.sin(a) * d, k: rand() });
  }
}
const trees = spots.filter(p => p.k > 0.35), rocks = spots.filter(p => p.k <= 0.35);
const trunks = new THREE.InstancedMesh(new THREE.CylinderGeometry(0.12, 0.18, 1.2, 6), new THREE.MeshStandardMaterial({ color: 0x6b4a2f, roughness: 1 }), trees.length);
const crowns = new THREE.InstancedMesh(new THREE.IcosahedronGeometry(0.9, 0), new THREE.MeshStandardMaterial({ roughness: 0.9, flatShading: true }), trees.length);
const stones = new THREE.InstancedMesh(new THREE.DodecahedronGeometry(0.45, 0), rockMat, rocks.length);
const m = new THREE.Object3D(), tint = new THREE.Color();
trees.forEach((p, i) => {
  const sc = 0.8 + p.k * 0.7;
  m.position.set(p.x, p.y + 0.6 * sc, p.z); m.scale.setScalar(sc); m.rotation.set(0, p.k * 9, 0); m.updateMatrix();
  trunks.setMatrixAt(i, m.matrix);
  m.position.y = p.y + 1.6 * sc; m.updateMatrix();
  crowns.setMatrixAt(i, m.matrix);
  crowns.setColorAt(i, tint.setHSL(0.27 + p.k * 0.08, 0.55, 0.36 + p.k * 0.12));
});
rocks.forEach((p, i) => {
  m.position.set(p.x, p.y + 0.15, p.z); m.scale.set(1 + p.k, 0.6 + p.k, 1); m.rotation.set(p.k, p.k * 7, 0); m.updateMatrix();
  stones.setMatrixAt(i, m.matrix);
});
for (const im of [trunks, crowns, stones]) { im.castShadow = im.receiveShadow = true; scene.add(im); }

// Drifting clouds, also instanced.
const puffs = [];
for (let c = 0; c < 14; c++) {
  // Above or below the islands, never through them.
  const cx = (rand() - 0.5) * 220, cy = rand() < 0.5 ? -28 + rand() * 10 : 18 + rand() * 16, cz = -40 + (rand() - 0.5) * 220;
  for (let k = 0; k < 5; k++) puffs.push({ x: cx + (k - 2) * 3.2, y: cy + rand() * 1.2, z: cz + (rand() - 0.5) * 2, s: 2.2 + rand() * 1.6, v: 0.6 + rand() * 0.6 });
}
const clouds = new THREE.InstancedMesh(new THREE.IcosahedronGeometry(1, 1), new THREE.MeshStandardMaterial({ color: 0xffffff, roughness: 1, transparent: true, opacity: 0.92 }), puffs.length);
scene.add(clouds);
// Place them now: an instance with no matrix sits at the origin, in the scene.
function driftClouds(dt) {
  puffs.forEach((c, i) => {
    c.x += c.v * dt; if (c.x > 130) c.x = -130;
    m.position.set(c.x, c.y, c.z); m.scale.setScalar(c.s); m.rotation.set(0, 0, 0); m.updateMatrix(); clouds.setMatrixAt(i, m.matrix);
  });
  clouds.instanceMatrix.needsUpdate = true;
}
driftClouds(0);

/* -------------------------------------------------------------------- gems */
const glowTex = (() => {
  const c = document.createElement('canvas'); c.width = c.height = 64;
  const g = c.getContext('2d'), grd = g.createRadialGradient(32, 32, 0, 32, 32, 32);
  grd.addColorStop(0, 'rgba(190,250,255,.85)'); grd.addColorStop(0.25, 'rgba(110,225,255,.45)'); grd.addColorStop(1, 'rgba(60,180,255,0)');
  g.fillStyle = grd; g.fillRect(0, 0, 64, 64);
  const t = new THREE.CanvasTexture(c); t.colorSpace = THREE.SRGBColorSpace; return t;
})();
const gemSpots = [[0.8, 1.2, -4], [-11, 2.8, -14], [7, 3.6, -21], [-7, 5.2, -33], [10, 4.4, -38], [2, 6.8, -47], [-12, 8.2, -52], [0, 9.4, -66]];
const gemMat = new THREE.MeshStandardMaterial({ color: 0x7ff3ff, emissive: 0x22b8d8, emissiveIntensity: 1.6, metalness: 0.2, roughness: 0.15 });
const gems = gemSpots.map(([x, y, z]) => {
  const mesh = new THREE.Mesh(new THREE.OctahedronGeometry(0.42), gemMat);
  mesh.castShadow = true;
  const glow = new THREE.Sprite(new THREE.SpriteMaterial({ map: glowTex, blending: THREE.AdditiveBlending, depthWrite: false, transparent: true }));
  glow.scale.setScalar(1.6);
  const g = new THREE.Group(); g.add(mesh, glow); g.position.set(x, y, z); scene.add(g);
  return { g, base: y, taken: false };
});

// One reusable particle burst for pickups.
const BURST = 40;
const burstGeo = new THREE.BufferGeometry();
burstGeo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(BURST * 3), 3));
const burstMat = new THREE.PointsMaterial({ color: 0x9ff6ff, size: 0.22, map: glowTex, blending: THREE.AdditiveBlending, depthWrite: false, transparent: true });
const burst = new THREE.Points(burstGeo, burstMat); burst.visible = false; burst.frustumCulled = false; scene.add(burst);
const burstVel = new Float32Array(BURST * 3); let burstLife = 0;
function pop(at) {
  const p = burstGeo.attributes.position.array;
  for (let i = 0; i < BURST; i++) {
    p[i * 3] = at.x; p[i * 3 + 1] = at.y; p[i * 3 + 2] = at.z;
    const a = rand() * Math.PI * 2, u = rand() * 2 - 1, s = 3 + rand() * 4, r = Math.sqrt(1 - u * u);
    burstVel[i * 3] = Math.cos(a) * r * s; burstVel[i * 3 + 1] = u * s + 2; burstVel[i * 3 + 2] = Math.sin(a) * r * s;
  }
  burstLife = 0.8; burst.visible = true;
}

/* ------------------------------------------------------------------- sound */
let audio = null;
function blip(freq, len = 0.12, type = 'sine', gain = 0.12) {
  try {
    audio = audio || new AudioContext();
    const o = audio.createOscillator(), g = audio.createGain(), t = audio.currentTime;
    o.type = type; o.frequency.setValueAtTime(freq, t); o.frequency.exponentialRampToValueAtTime(freq * 1.6, t + len);
    g.gain.setValueAtTime(gain, t); g.gain.exponentialRampToValueAtTime(0.0001, t + len);
    o.connect(g).connect(audio.destination); o.start(t); o.stop(t + len);
  } catch {}
}

/* -------------------------------------------------------------------- hero */
const loader = new GLTFLoader();
const hero = { root: new THREE.Group(), model: null, mixer: null, actions: {}, current: null };
scene.add(hero.root);
const shadowBlob = new THREE.Mesh(new THREE.CircleGeometry(0.5, 20), new THREE.MeshBasicMaterial({ color: 0x000000, transparent: true, opacity: 0.22, depthWrite: false }));
shadowBlob.rotation.x = -Math.PI / 2; scene.add(shadowBlob);

async function heroes() {
  try {
    const r = await fetch('assets/characters/characters.json');
    const list = await r.json();
    if (Array.isArray(list) && list.length) return list;
  } catch {}
  return [{ name: 'Robot', file: 'robot.glb' }];
}

/** Load a hero: 1.7 m tall, feet on the ground, centred, facing +Z
 *  (`yaw` in characters.json turns a model that faces another way). */
async function loadModel(entry) {
  const gltf = await loader.loadAsync(`assets/characters/${entry.file}`);
  const inner = gltf.scene;
  inner.traverse(o => { if (o.isMesh) { o.castShadow = true; o.receiveShadow = true; } });
  // World matrices first: a glTF often scales its own nodes, and a box
  // measured without them sizes the hero wrongly.
  inner.updateMatrixWorld(true);
  const box = new THREE.Box3().setFromObject(inner), size = box.getSize(new THREE.Vector3());
  const scale = 1.7 / (size.y || 1);
  inner.scale.multiplyScalar(scale);
  inner.updateMatrixWorld(true);
  box.setFromObject(inner);
  const centre = box.getCenter(new THREE.Vector3());
  inner.position.set(-centre.x, -box.min.y, -centre.z);
  const model = new THREE.Group();
  model.add(inner);
  model.rotation.y = entry.yaw || 0;
  return { model, animations: gltf.animations, footprint: Math.max(size.x, size.z) * scale };
}

async function loadHero(entry) {
  const { model, animations, footprint } = await loadModel(entry);
  if (hero.model) hero.root.remove(hero.model);
  hero.mixer = null; hero.actions = {}; hero.current = null;
  const gltf = { animations };
  hero.model = model; hero.root.add(model);
  shadowBlob.scale.setScalar(footprint * 0.9);
  if (gltf.animations.length) {
    hero.mixer = new THREE.AnimationMixer(model);
    const find = re => gltf.animations.find(a => re.test(a.name));
    for (const [key, re] of [['idle', /idle/i], ['run', /run/i], ['walk', /walk/i], ['jump', /jump/i]]) {
      const clip = find(re);
      if (clip) hero.actions[key] = hero.mixer.clipAction(clip);
    }
    if (hero.actions.jump) { hero.actions.jump.setLoop(THREE.LoopOnce, 1); hero.actions.jump.clampWhenFinished = true; }
    play('idle');
  }
}

function play(name, fade = 0.2) {
  const next = hero.actions[name] || hero.actions.walk || hero.actions.idle;
  if (!next || next === hero.current) return;
  next.reset().fadeIn(fade).play();
  if (hero.current) hero.current.fadeOut(fade);
  hero.current = next;
}

/* ------------------------------------------------------------------- input */
const keys = new Set();
const stick = { x: 0, y: 0, id: null };
let jumpPressed = false, started = false, menuChosen = null;
addEventListener('keydown', e => {
  if (['Space', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(e.code)) e.preventDefault();
  if (!started && !$('menu').hidden) { start(); }
  if (e.code === 'Space' && !e.repeat) jumpPressed = true;
  keys.add(e.code);
});
addEventListener('keyup', e => keys.delete(e.code));
const stickEl = $('stick'), knob = $('knob');
stickEl.addEventListener('pointerdown', e => { stick.id = e.pointerId; stickEl.setPointerCapture(e.pointerId); moveStick(e); });
stickEl.addEventListener('pointermove', e => { if (e.pointerId === stick.id) moveStick(e); });
const release = e => { if (e.pointerId !== stick.id) return; stick.id = null; stick.x = stick.y = 0; knob.style.transform = ''; };
stickEl.addEventListener('pointerup', release);
stickEl.addEventListener('pointercancel', release);
function moveStick(e) {
  const r = stickEl.getBoundingClientRect();
  let dx = (e.clientX - (r.left + r.width / 2)) / (r.width / 2), dy = (e.clientY - (r.top + r.height / 2)) / (r.height / 2);
  const len = Math.hypot(dx, dy); if (len > 1) { dx /= len; dy /= len; }
  stick.x = dx; stick.y = dy;
  knob.style.transform = `translate(${dx * 36}px, ${dy * 36}px)`;
}
$('jump').addEventListener('pointerdown', e => { e.preventDefault(); jumpPressed = true; });

/* ------------------------------------------------------------------- state */
const S = { pos: new THREE.Vector3(0, 0, 6), vel: new THREE.Vector3(), yaw: Math.PI, grounded: true, coyote: 0, buffer: 0,
  jumps: 0, safe: new THREE.Vector3(0, 0, 6), squash: 0, time: 0, collected: 0, finished: false };

function floorAt(x, z, y) {
  let best = -Infinity;
  for (const s of islands) {
    if (Math.hypot(x - s.x, z - s.z) < s.r * 0.97 && y >= s.top - 0.45) best = Math.max(best, s.top);
  }
  return best;
}

function reset() {
  S.pos.set(0, 0, 6); S.vel.set(0, 0, 0); S.safe.copy(S.pos); S.time = 0; S.collected = 0; S.finished = false;
  for (const g of gems) { g.taken = false; g.g.visible = true; }
  $('done').hidden = true;
}

function step(dt) {
  if (!S.finished) S.time += dt;
  let ix = (keys.has('KeyD') || keys.has('ArrowRight') ? 1 : 0) - (keys.has('KeyA') || keys.has('ArrowLeft') ? 1 : 0) + stick.x;
  let iz = (keys.has('KeyS') || keys.has('ArrowDown') ? 1 : 0) - (keys.has('KeyW') || keys.has('ArrowUp') ? 1 : 0) + stick.y;
  const len = Math.hypot(ix, iz); if (len > 1) { ix /= len; iz /= len; }
  const speed = 7.5;
  const k = S.grounded ? 14 : 5;
  S.vel.x = THREE.MathUtils.damp(S.vel.x, ix * speed, k, dt);
  S.vel.z = THREE.MathUtils.damp(S.vel.z, iz * speed, k, dt);

  // Jump buffering and coyote time make jumps forgiving; a second jump in the air.
  S.buffer = jumpPressed ? 0.14 : Math.max(0, S.buffer - dt);
  jumpPressed = false;
  if (S.buffer > 0 && (S.grounded || S.coyote > 0)) { S.vel.y = 9.6; S.grounded = false; S.coyote = 0; S.buffer = 0; S.jumps = 1; blip(330, 0.1, 'triangle', 0.08); play('jump', 0.08); }
  else if (S.buffer > 0 && S.jumps === 1) { S.vel.y = 8.4; S.jumps = 2; S.buffer = 0; blip(495, 0.1, 'triangle', 0.08); play('jump', 0.05); }

  S.vel.y -= 26 * dt;
  S.pos.addScaledVector(S.vel, dt);
  const floor = floorAt(S.pos.x, S.pos.z, S.pos.y);
  if (S.pos.y <= floor && S.vel.y <= 0) {
    if (!S.grounded && S.vel.y < -6) S.squash = 0.25;
    S.pos.y = floor; S.vel.y = 0; S.grounded = true; S.jumps = 0; S.coyote = 0.12; S.safe.copy(S.pos);
  } else {
    if (S.grounded) S.coyote = 0.12;
    S.grounded = false; S.coyote = Math.max(0, S.coyote - dt);
  }
  if (S.pos.y < -30) { S.pos.copy(S.safe).y += 2; S.vel.set(0, 0, 0); blip(150, 0.25, 'sawtooth', 0.06); }

  const moving = Math.hypot(S.vel.x, S.vel.z);
  if (moving > 0.3) S.yaw = Math.atan2(S.vel.x, S.vel.z);
  hero.root.position.copy(S.pos);
  const q = new THREE.Quaternion().setFromAxisAngle(THREE.Object3D.DEFAULT_UP, S.yaw);
  hero.root.quaternion.slerp(q, 1 - Math.exp(-14 * dt));

  // Animation: clips when the hero has them, otherwise a procedural run.
  if (hero.mixer) {
    if (S.grounded) play(moving > 0.6 ? 'run' : 'idle');
    if (hero.actions.run) hero.actions.run.timeScale = THREE.MathUtils.clamp(moving / 6, 0.6, 1.4);
    hero.mixer.update(dt);
  } else if (hero.model) {
    // A static model runs by bobbing, rocking and leaning into the motion.
    const t = S.time, run = Math.min(1, moving / 4);
    const inner = hero.model.children[0];
    inner.userData.y ??= inner.position.y;
    inner.position.y = inner.userData.y + (S.grounded ? Math.abs(Math.sin(t * 13)) * 0.09 * run : 0);
    hero.model.rotation.z = Math.sin(t * 13) * 0.06 * run;
  }
  S.squash = Math.max(0, S.squash - dt);
  const sq = S.squash * 1.2;
  hero.root.scale.set(1 + sq * 0.5, 1 - sq, 1 + sq * 0.5);
  if (!S.grounded) hero.root.scale.set(0.96, 1.06, 0.96);

  shadowBlob.position.set(S.pos.x, (floor > -Infinity ? floor : S.pos.y) + 0.02, S.pos.z);
  shadowBlob.visible = floor > -Infinity;

  // Gems.
  const t = performance.now() / 1000;
  for (const g of gems) {
    if (g.taken) continue;
    g.g.position.y = g.base + Math.sin(t * 2 + g.base) * 0.2;
    g.g.children[0].rotation.y = t * 1.8;
    if (g.g.position.distanceTo(tmpV.copy(S.pos).setY(S.pos.y + 0.9)) < 1.2) {
      g.taken = true; g.g.visible = false; S.collected++; pop(g.g.position); blip(880, 0.16, 'sine', 0.14);
      if (S.collected === gems.length) finish();
    }
  }
  if (burstLife > 0) {
    burstLife -= dt;
    const p = burstGeo.attributes.position.array;
    for (let i = 0; i < BURST * 3; i += 3) {
      burstVel[i + 1] -= 9 * dt;
      p[i] += burstVel[i] * dt; p[i + 1] += burstVel[i + 1] * dt; p[i + 2] += burstVel[i + 2] * dt;
    }
    burstGeo.attributes.position.needsUpdate = true;
    burstMat.opacity = Math.max(0, burstLife / 0.8);
    if (burstLife <= 0) burst.visible = false;
  }

  // Clouds drift; camera follows with a little look-ahead; the sun follows too.
  driftClouds(dt);
  camGoal.set(S.pos.x + S.vel.x * 0.25, S.pos.y + 4.4, S.pos.z + 9.5 + S.vel.z * 0.15);
  camera.position.lerp(camGoal, 1 - Math.exp(-5 * dt));
  camera.lookAt(S.pos.x, S.pos.y + 1.2, S.pos.z - 2);
  sun.position.set(S.pos.x + 12, S.pos.y + 22, S.pos.z + 8);
  sun.target.position.copy(S.pos);

  $('gems').textContent = `Gems ${S.collected} / ${gems.length}`;
  $('clock').textContent = fmt(S.time);
}
const tmpV = new THREE.Vector3(), camGoal = new THREE.Vector3();
const fmt = s => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;

function finish() {
  S.finished = true;
  $('result').textContent = `${gems.length} gems in ${fmt(S.time)}. Can you beat it?`;
  $('done').hidden = false;
  blip(660, 0.3, 'sine', 0.12); setTimeout(() => blip(990, 0.4, 'sine', 0.12), 160);
  if (hero.actions.idle) play('idle');
}
$('again').onclick = () => reset();

/* --------------------------------------------------------- the line-up */
// Every hero stands together on the first island while the menu is open.
const lineup = new THREE.Group(); scene.add(lineup);
const lineupMixers = [];
async function showLineup(list) {
  const n = list.length;
  for (let i = 0; i < n; i++) {
    try {
      const { model, animations } = await loadModel(list[i]);
      const slot = new THREE.Group();
      slot.position.set((i - (n - 1) / 2) * 1.85, 0, 1.5 - Math.abs(i - (n - 1) / 2) * 0.5);
      slot.add(model); slot.userData.entry = list[i];
      lineup.add(slot);
      if (animations.length) {
        const mixer = new THREE.AnimationMixer(model);
        const pick = animations.find(a => /wave/i.test(a.name)) || animations.find(a => /idle/i.test(a.name)) || animations[0];
        mixer.clipAction(pick).play();
        lineupMixers.push(mixer);
      }
    } catch (e) { console.warn('Could not load', list[i].file, e); }
  }
}

/* -------------------------------------------------------------- start-up */
function start() {
  if (started) return;
  started = true;
  $('menu').hidden = true;
  lineup.visible = false;
  hero.root.visible = true;
  try { audio = audio || new AudioContext(); audio.resume?.(); } catch {}
}
$('play').onclick = start;

(async () => {
  const list = await heroes();
  const box = $('heroes');
  let chosen = list[0];
  const draw = () => {
    box.replaceChildren(...list.map(h => {
      const b = document.createElement('button');
      b.textContent = h.name; b.setAttribute('aria-pressed', String(h === chosen));
      b.onclick = async () => { chosen = h; draw(); await loadHero(h).catch(() => {}); };
      return b;
    }));
  };
  draw();
  hero.root.visible = false;
  menuChosen = () => chosen;
  await Promise.all([showLineup(list), loadHero(chosen).catch(err => console.warn('Could not load the hero', err))]);
})();

function resize() {
  renderer.setSize(innerWidth, innerHeight, false);
  camera.aspect = innerWidth / innerHeight;
  camera.updateProjectionMatrix();
}
addEventListener('resize', resize);
resize();
camera.position.set(0, 5, 16);

let paused = false;
const clock = new THREE.Clock();
document.addEventListener('visibilitychange', () => { paused = document.hidden; clock.getDelta(); });
renderer.setAnimationLoop(() => {
  const dt = Math.min(0.05, clock.getDelta());
  if (!paused) {
    if (started) step(dt);
    else {
      // The menu: a slow drift in front of the line-up; the chosen hero turns.
      const t = performance.now() / 1000;
      camera.position.set(Math.sin(t * 0.25) * 1.4, 1.55, 6.3);
      camera.lookAt(0, 1.2, 1);
      for (const mx of lineupMixers) mx.update(dt);
      driftClouds(dt);
      const pick = menuChosen && menuChosen();
      for (const slot of lineup.children) {
        const on = pick && slot.userData.entry === pick;
        slot.rotation.y = THREE.MathUtils.damp(slot.rotation.y, on ? Math.sin(t * 1.5) * 0.5 : 0, 6, dt);
        slot.position.y = THREE.MathUtils.damp(slot.position.y, on ? 0.15 : 0, 6, dt);
      }
      sun.position.set(6, 14, 10); sun.target.position.set(0, 0, 0);
    }
  }
  renderer.render(scene, camera);
});
