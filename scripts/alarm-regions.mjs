// Builds the air raid alerts' region table (src-tauri/src/apps/alarm/regions.tsv) from the
// public map's region geometry: every oblast, district and community with its parent. The
// geometry has ids and names but no parents, so a region's parent is the region of the level
// above that holds a point inside it.
//
//   node scripts/alarm-regions.mjs [geometry.geojson]
//
// Without a file it downloads the geometry (about 8 MB). Run it again when the service changes
// its regions; the table is compiled into the island.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SOURCE = 'https://map.ukrainealarm.com/data/real-map/regions-standard-v1.geojson';
const OUT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'src-tauri', 'src', 'apps', 'alarm', 'regions.tsv');

const text = process.argv[2]
  ? fs.readFileSync(process.argv[2], 'utf8')
  : await (await fetch(SOURCE, { headers: { 'User-Agent': 'GreatIsland' } })).text();
const features = JSON.parse(text).features;

const polygons = (f) => (f.geometry.type === 'Polygon' ? [f.geometry.coordinates] : f.geometry.coordinates);

const inRing = ([x, y], ring) => {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, yi] = ring[i];
    const [xj, yj] = ring[j];
    if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
};

const inside = (point, f) => polygons(f).some((p) => inRing(point, p[0]) && !p.slice(1).some((hole) => inRing(point, hole)));

// A point surely inside: the middle of the widest stretch of a few horizontal lines across the
// largest polygon (a centre may fall outside a bent region or into a city's hole).
function innerPoint(f) {
  let best = null;
  let bestWidth = 0;
  const outer = polygons(f)
    .map((p) => p[0])
    .sort((a, b) => b.length - a.length)[0];
  const ys = outer.map((p) => p[1]);
  const [lo, hi] = [Math.min(...ys), Math.max(...ys)];
  for (let k = 1; k < 10; k++) {
    const y = lo + ((hi - lo) * k) / 10;
    const xs = [];
    for (let i = 0, j = outer.length - 1; i < outer.length; j = i++) {
      const [xi, yi] = outer[i];
      const [xj, yj] = outer[j];
      if (yi > y !== yj > y) xs.push(((xj - xi) * (y - yi)) / (yj - yi) + xi);
    }
    xs.sort((a, b) => a - b);
    for (let i = 0; i + 1 < xs.length; i += 2) {
      const point = [(xs[i] + xs[i + 1]) / 2, y];
      if (xs[i + 1] - xs[i] > bestWidth && inside(point, f)) {
        bestWidth = xs[i + 1] - xs[i];
        best = point;
      }
    }
  }
  return best ?? outer[0];
}

const byRank = (rank) => features.filter((f) => f.properties.rank === rank);
const [oblasts, districts, communities] = [byRank(1), byRank(2), byRank(3)];
// A city counted apart (Kharkiv, Zaporizhzhia) is a hole in its district: then the district
// whose outline holds it. An alert of the district or the oblast reaches the city too: for an
// alarm a needless notice is better than a missed one.
const withinOutline = (point, f) => polygons(f).some((p) => inRing(point, p[0]));
const parentOf = (f, pool) => {
  const point = innerPoint(f);
  return pool.find((p) => inside(point, p)) ?? pool.find((p) => withinOutline(point, p));
};

const rows = [];
const missing = [];
for (const f of oblasts) rows.push([f.properties.regionId, '0', f.properties.name]);
for (const [pool, children] of [
  [oblasts, districts],
  [districts, communities],
]) {
  for (const f of children) {
    const parent = parentOf(f, pool);
    if (!parent) missing.push(`${f.properties.regionId} ${f.properties.name}`);
    else rows.push([f.properties.regionId, parent.properties.regionId, f.properties.name]);
  }
}
if (missing.length) {
  console.error('no parent found for:\n  ' + missing.join('\n  '));
  process.exit(1);
}
rows.sort((a, b) => Number(a[0]) - Number(b[0]));
fs.mkdirSync(path.dirname(OUT), { recursive: true });
fs.writeFileSync(OUT, rows.map((r) => r.join('\t')).join('\n') + '\n');
console.log(`${rows.length} regions: ${oblasts.length} oblast-level, ${districts.length} districts, ${communities.length} communities -> ${OUT}`);
