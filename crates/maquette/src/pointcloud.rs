use crate::config::RenderConfig;
use crate::math::{FloatExt, FxBuildHasher, Vec3};
use crate::parser::Triangle;
use crate::ply_parser::PointCloud;
use crate::projection::resolve_config_view;
use crate::render::{bbox_center, bbox_radius};
use std::collections::HashMap;

pub(crate) struct Grid {
    origin: Vec3,
    inv: f64,
    h: f64,
    mask: u32,
    start: Vec<u32>,
    items: Vec<u32>,
    xyz: Vec<[f32; 3]>,
}

pub(crate) struct Query {
    seen: Vec<u32>,
    stamp: u32,
    k: usize,
    out: Vec<(u32, f32)>,
}

impl Grid {
    pub(crate) fn new(positions: &[Vec3], origin: Vec3, h: f64) -> Self {
        let size = (positions.len() * 2).max(16).next_power_of_two();
        let mask = (size - 1) as u32;
        let mut g = Grid { origin, inv: 1.0 / h, h, mask, start: vec![0; size + 1], items: vec![0; positions.len()], xyz: vec![[0.0; 3]; positions.len()] };
        let buckets: Vec<u32> = positions.iter().map(|&p| g.bucket(g.cell(p))).collect();
        for &b in &buckets { g.start[b as usize + 1] += 1; }
        for i in 0..size { g.start[i + 1] += g.start[i]; }
        let mut fill = g.start.clone();
        for (i, &b) in buckets.iter().enumerate() {
            let slot = fill[b as usize] as usize;
            g.items[slot] = i as u32;
            let p = positions[i];
            g.xyz[slot] = [p.x as f32, p.y as f32, p.z as f32];
            fill[b as usize] += 1;
        }
        g
    }

    pub(crate) fn query(&self) -> Query {
        Query { seen: vec![u32::MAX; self.start.len() - 1], stamp: 0, k: 1, out: Vec::with_capacity(64) }
    }

    #[inline]
    fn cell(&self, p: Vec3) -> (i32, i32, i32) {
        (
            ((p.x - self.origin.x) * self.inv).floor() as i32,
            ((p.y - self.origin.y) * self.inv).floor() as i32,
            ((p.z - self.origin.z) * self.inv).floor() as i32,
        )
    }

    #[inline]
    fn bucket(&self, c: (i32, i32, i32)) -> u32 {
        ((c.0 as u32).wrapping_mul(73_856_093) ^ (c.1 as u32).wrapping_mul(19_349_663) ^ (c.2 as u32).wrapping_mul(83_492_791)) & self.mask
    }

    #[inline]
    fn scan(&self, b: u32, i: u32, p: [f32; 3], q: &mut Query) {
        if q.seen[b as usize] == q.stamp { return; }
        q.seen[b as usize] = q.stamp;
        for slot in self.start[b as usize] as usize..self.start[b as usize + 1] as usize {
            let j = self.items[slot];
            if j == i { continue; }
            let c = self.xyz[slot];
            let (dx, dy, dz) = (c[0] - p[0], c[1] - p[1], c[2] - p[2]);
            let d = dx * dx + dy * dy + dz * dz;
            let len = q.out.len();
            if len == q.k {
                if d >= q.out[len - 1].1 { continue; }
                q.out.pop();
            }
            let mut at = q.out.len();
            while at > 0 && q.out[at - 1].1 > d { at -= 1; }
            q.out.insert(at, (j, d));
        }
    }

    pub(crate) fn knn(&self, positions: &[Vec3], i: usize, k: usize, max_ring: i32, q: &mut Query) {
        let pv = positions[i];
        let p = [pv.x as f32, pv.y as f32, pv.z as f32];
        let c = self.cell(pv);
        q.stamp = q.stamp.wrapping_add(1);
        if q.stamp == u32::MAX {
            q.seen.iter_mut().for_each(|s| *s = u32::MAX);
            q.stamp = 0;
        }
        q.out.clear();
        q.k = k.max(1);
        self.scan(self.bucket(c), i as u32, p, q);
        let mut ring: i32 = 1;
        loop {
            for dz in -ring..=ring {
                for dy in -ring..=ring {
                    let edge = dz.abs() == ring || dy.abs() == ring;
                    let mut dx = -ring;
                    while dx <= ring {
                        self.scan(self.bucket((c.0 + dx, c.1 + dy, c.2 + dz)), i as u32, p, q);
                        dx += if edge || dx == ring { 1 } else { 2 * ring };
                    }
                }
            }
            let reach = (ring as f64 * self.h) as f32;
            if ring >= max_ring || (q.out.len() == q.k && q.out[q.k - 1].1 <= reach * reach) { return; }
            ring += 1;
        }
    }
}

pub(crate) struct Neighbours {
    pub(crate) k: usize,
    pub(crate) nbr: Vec<u32>,
    pub(crate) dsq: Vec<f32>,
    pub(crate) cnt: Vec<u8>,
}

impl Neighbours {
    pub(crate) fn of(&self, i: usize) -> &[u32] { &self.nbr[i * self.k..i * self.k + self.cnt[i] as usize] }
    pub(crate) fn dsq_of(&self, i: usize) -> &[f32] { &self.dsq[i * self.k..i * self.k + self.cnt[i] as usize] }
}

pub(crate) fn neighbours(positions: &[Vec3], bmin: Vec3, bmax: Vec3, k: usize) -> Neighbours {
    let n = positions.len();
    let k = k.min(n.saturating_sub(1)).min(255);
    if k == 0 { return Neighbours { k: 1, nbr: vec![0; n], dsq: vec![0.0; n], cnt: vec![0; n] }; }
    let diag = bmax.sub(bmin).length();
    let guess = (diag / (n as f64).sqrt() * 2.0).fmax(diag * 1e-6);
    let coarse = Grid::new(positions, bmin, guess);
    let mut q = coarse.query();
    let step = (n / 256).max(1);
    let mut kth: Vec<f32> = (0..n)
        .step_by(step)
        .filter_map(|i| {
            coarse.knn(positions, i, k, 12, &mut q);
            (q.out.len() == k).then(|| q.out[k - 1].1.sqrt())
        })
        .collect();
    let h = if kth.is_empty() {
        guess
    } else {
        let at = kth.len() * 17 / 20;
        kth.select_nth_unstable_by(at, f32::total_cmp);
        (kth[at] as f64).fmax(diag * 1e-6)
    };
    let grid = Grid::new(positions, bmin, h);
    let mut q = grid.query();
    let mut nb = Neighbours { k, nbr: vec![0; n * k], dsq: vec![0.0; n * k], cnt: vec![0; n] };
    for i in 0..n {
        grid.knn(positions, i, k, 3, &mut q);
        nb.cnt[i] = q.out.len() as u8;
        for (s, &(j, d)) in q.out.iter().enumerate() {
            nb.nbr[i * k + s] = j;
            nb.dsq[i * k + s] = d;
        }
    }
    nb
}

pub(crate) fn smallest_eigvec_sym3(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> Vec3 {
    let p1 = d * d + e * e + f * f;
    if p1 <= 1e-20 {
        return if a <= b && a <= c {
            Vec3::new(1.0, 0.0, 0.0)
        } else if b <= c {
            Vec3::new(0.0, 1.0, 0.0)
        } else {
            Vec3::new(0.0, 0.0, 1.0)
        };
    }
    let q = (a + b + c) / 3.0;
    let p2 = (a - q).powi(2) + (b - q).powi(2) + (c - q).powi(2) + 2.0 * p1;
    let pp = (p2 / 6.0).sqrt();
    let (ba, bb, bc) = ((a - q) / pp, (b - q) / pp, (c - q) / pp);
    let (bd, be, bf) = (d / pp, e / pp, f / pp);
    let detb = ba * (bb * bc - bf * bf) - bd * (bd * bc - bf * be) + be * (bd * bf - bb * be);
    let r = (detb / 2.0).clamp(-1.0, 1.0);
    let phi = r.acos() / 3.0;
    let two_pp = 2.0 * pp;
    let e1 = q + two_pp * phi.cos();
    let e2 = q + two_pp * (phi + 2.0 * std::f64::consts::FRAC_PI_3).cos();
    let e3 = 3.0 * q - e1 - e2;
    let lambda = e1.fmin(e2).fmin(e3);
    let r0 = Vec3::new(a - lambda, d, e);
    let r1 = Vec3::new(d, b - lambda, f);
    let r2 = Vec3::new(e, f, c - lambda);
    let cands = [r0.cross(r1), r1.cross(r2), r2.cross(r0)];
    let mut best = cands[0];
    for c in &cands[1..] {
        if c.dot(*c) > best.dot(best) { best = *c; }
    }
    if best.dot(best) < 1e-24 { Vec3::new(0.0, 0.0, 1.0) } else { best.normalized() }
}

pub(crate) fn pca_normal(p: Vec3, nbr: &[u32], positions: &[Vec3], eye: Vec3) -> Vec3 {
    let to_eye = eye.sub(p);
    if nbr.len() < 2 { return to_eye.normalized(); }
    let mut c = p;
    for &j in nbr { c = c.add(positions[j as usize]); }
    let c = c.scale(1.0 / (nbr.len() as f64 + 1.0));
    let (mut a, mut b, mut cc, mut d, mut e, mut f) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for q in std::iter::once(p).chain(nbr.iter().map(|&j| positions[j as usize])) {
        let (x, y, z) = (q.x - c.x, q.y - c.y, q.z - c.z);
        a += x * x;
        b += y * y;
        cc += z * z;
        d += x * y;
        e += x * z;
        f += y * z;
    }
    let nrm = smallest_eigvec_sym3(a, b, cc, d, e, f);
    if nrm.dot(to_eye) < 0.0 { nrm.scale(-1.0) } else { nrm }
}

pub(crate) fn orient_normals(normals: &mut [Vec3], positions: &[Vec3], nb: &Neighbours, eye: Vec3) {
    let n = normals.len();
    let mut order: Vec<(f64, u32)> = (0..n).map(|i| { let e = positions[i].sub(eye); (e.dot(e), i as u32) }).collect();
    order.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut visited = vec![false; n];
    let mut comp: Vec<u32> = Vec::new();
    let mut queue: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
    for &(_, seed) in &order {
        if visited[seed as usize] { continue; }
        let s = seed as usize;
        if normals[s].dot(eye.sub(positions[s])) < 0.0 { normals[s] = normals[s].scale(-1.0); }
        visited[s] = true;
        comp.clear();
        comp.push(seed);
        for thr in [0.8, 0.3, -1.0] {
            queue.clear();
            queue.extend(comp.iter().copied());
            while let Some(i) = queue.pop_front() {
                let ni = normals[i as usize];
                for &j in nb.of(i as usize) {
                    let ju = j as usize;
                    if visited[ju] { continue; }
                    let d = ni.dot(normals[ju]);
                    if d.abs() < thr { continue; }
                    if d < 0.0 { normals[ju] = normals[ju].scale(-1.0); }
                    visited[ju] = true;
                    comp.push(j);
                    queue.push_back(j);
                }
            }
        }
    }
}

pub(crate) fn eye_for(config: &RenderConfig, bmin: Vec3, bmax: Vec3) -> Vec3 {
    resolve_config_view(config, bbox_center(bmin, bmax), bbox_radius(bmin, bmax)).camera
}

struct Cell {
    pts: Vec<(f64, f64)>,
    own: Vec<i32>,
    next_pts: Vec<(f64, f64)>,
    next_own: Vec<i32>,
}

impl Cell {
    fn reset(&mut self, l: f64) {
        self.pts.clear();
        self.own.clear();
        self.pts.extend_from_slice(&[(-l, -l), (l, -l), (l, l), (-l, l)]);
        self.own.extend_from_slice(&[-1, -1, -1, -1]);
    }

    fn clip(&mut self, a: (f64, f64), owner: i32) {
        let c = 0.5 * (a.0 * a.0 + a.1 * a.1);
        let m = self.pts.len();
        let side = |p: (f64, f64)| a.0 * p.0 + a.1 * p.1 - c;
        if self.pts.iter().all(|&p| side(p) <= 0.0) { return; }
        self.next_pts.clear();
        self.next_own.clear();
        for k in 0..m {
            let p = self.pts[k];
            let q = self.pts[(k + 1) % m];
            let (sp, sq) = (side(p), side(q));
            if sp <= 0.0 {
                self.next_pts.push(p);
                self.next_own.push(self.own[k]);
                if sq > 0.0 {
                    let t = sp / (sp - sq);
                    self.next_pts.push((p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1)));
                    self.next_own.push(owner);
                }
            } else if sq <= 0.0 {
                let t = sp / (sp - sq);
                self.next_pts.push((p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1)));
                self.next_own.push(self.own[k]);
            }
        }
        std::mem::swap(&mut self.pts, &mut self.next_pts);
        std::mem::swap(&mut self.own, &mut self.next_own);
    }
}

fn dedup(cloud: &PointCloud) -> Option<PointCloud> {
    let n = cloud.positions.len();
    let key = |i: usize| {
        let p = cloud.positions[i];
        (p.x.to_bits(), p.y.to_bits(), p.z.to_bits())
    };
    let mut order: Vec<u32> = (0..n as u32).collect();
    order.sort_unstable_by_key(|&i| (key(i as usize), i));
    let mut keep = vec![true; n];
    let mut any = false;
    for w in order.windows(2) {
        if key(w[0] as usize) == key(w[1] as usize) {
            keep[w[1] as usize] = false;
            any = true;
        }
    }
    if !any { return None; }
    let pick = |len: usize| -> Vec<usize> { if len == n { (0..n).filter(|&i| keep[i]).collect() } else { Vec::new() } };
    Some(PointCloud {
        positions: pick(n).into_iter().map(|i| cloud.positions[i]).collect(),
        normals: pick(cloud.normals.len()).into_iter().map(|i| cloud.normals[i]).collect(),
        colors: pick(cloud.colors.len()).into_iter().map(|i| cloud.colors[i]).collect(),
        scalars: pick(cloud.scalars.len()).into_iter().map(|i| cloud.scalars[i]).collect(),
    })
}

pub(crate) fn reconstruct(cloud: &PointCloud, config: &RenderConfig) -> Vec<Triangle> {
    let unique = dedup(cloud);
    let cloud = unique.as_ref().unwrap_or(cloud);
    let n = cloud.positions.len();
    if n < 3 { return Vec::new(); }
    let positions = &cloud.positions;
    let (bmin, bmax) = crate::render::bbox_of(positions.iter().copied());
    if bmax.sub(bmin).length() < 1e-12 { return Vec::new(); }

    let has_normals = cloud.normals.len() == n;
    let has_colors = cloud.colors.len() == n;
    let has_scalars = cloud.scalars.len() == n;
    let k = config.point_neighbors.max(3);
    let boundary_cos = if config.point_boundary > 0.0 { config.point_boundary.to_radians().cos() } else { -2.0 };
    let max_dsq = if config.point_size > 0.0 { config.point_size * config.point_size } else { f64::INFINITY };

    let nb = neighbours(positions, bmin, bmax, k);
    let normals: Vec<Vec3> = if has_normals {
        cloud.normals.iter().map(|v| v.normalized()).collect()
    } else {
        let eye = eye_for(config, bmin, bmax);
        let mut nv: Vec<Vec3> = (0..n).map(|i| pca_normal(positions[i], nb.of(i), positions, eye)).collect();
        orient_normals(&mut nv, positions, &nb, eye);
        nv
    };
    let colors: Vec<(u8, u8, u8)> = if has_colors && config.point_denoise {
        (0..n).map(|i| denoise_color(i, &nb, &cloud.colors)).collect()
    } else if has_colors {
        cloud.colors.clone()
    } else {
        Vec::new()
    };

    let mut votes: HashMap<(u32, u32, u32), u8, FxBuildHasher> =
        HashMap::with_capacity_and_hasher(n * 4, FxBuildHasher::default());
    let mut cell = Cell { pts: Vec::with_capacity(32), own: Vec::with_capacity(32), next_pts: Vec::with_capacity(32), next_own: Vec::with_capacity(32) };
    let mut local: Vec<(u32, (f64, f64))> = Vec::with_capacity(k);
    for i in 0..n {
        let p = positions[i];
        let ni = normals[i];
        let (t1, t2) = ni.tangent_basis();
        local.clear();
        let mut reach = 0.0f64;
        let ds = nb.dsq_of(i);
        let far = if ds.is_empty() { 0.0 } else { 4.0 * ds[ds.len() / 2] as f64 };
        for (&j, &d) in nb.of(i).iter().zip(ds) {
            let d = d as f64;
            if d > max_dsq || d > far || d < 1e-24 { continue; }
            if normals[j as usize].dot(ni) < boundary_cos { continue; }
            let e = positions[j as usize].sub(p);
            let uv = (e.dot(t1), e.dot(t2));
            if uv.0 * uv.0 + uv.1 * uv.1 < 1e-24 { continue; }
            reach = reach.fmax(d);
            local.push((j, uv));
        }
        if local.len() < 2 { continue; }
        cell.reset(reach.sqrt());
        for (s, &(_, uv)) in local.iter().enumerate() {
            cell.clip(uv, s as i32);
        }
        let m = cell.own.len();
        for e in 0..m {
            let (oa, ob) = (cell.own[e], cell.own[(e + 1) % m]);
            if oa < 0 || ob < 0 || oa == ob { continue; }
            let (mut a, mut b, mut c) = (i as u32, local[oa as usize].0, local[ob as usize].0);
            if a > b { std::mem::swap(&mut a, &mut b); }
            if b > c { std::mem::swap(&mut b, &mut c); }
            if a > b { std::mem::swap(&mut a, &mut b); }
            *votes.entry((a, b, c)).or_insert(0) += 1;
        }
    }

    let mut cand: Vec<(u8, f64, (u32, u32, u32))> = votes
        .iter()
        .map(|(&t, &v)| {
            let (pa, pb, pc) = (positions[t.0 as usize], positions[t.1 as usize], positions[t.2 as usize]);
            let longest = pb.sub(pa).dot(pb.sub(pa)).fmax(pc.sub(pb).dot(pc.sub(pb))).fmax(pa.sub(pc).dot(pa.sub(pc)));
            (v, longest, t)
        })
        .collect();
    cand.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut edges: HashMap<(u32, u32), u8, FxBuildHasher> =
        HashMap::with_capacity_and_hasher(cand.len() * 2, FxBuildHasher::default());
    let mut strong: Vec<(u32, u32, u32)> = Vec::with_capacity(cand.len());
    for &(v, _, (a, b, c)) in &cand {
        let use_of = |e: (u32, u32)| edges.get(&e).copied().unwrap_or(0);
        let (ab, bc, ac) = (use_of((a, b)), use_of((b, c)), use_of((a, c)));
        if ab >= 2 || bc >= 2 || ac >= 2 || (v < 2 && ab + bc + ac == 0) { continue; }
        for e in [(a, b), (b, c), (a, c)] { *edges.entry(e).or_insert(0) += 1; }
        strong.push((a, b, c));
    }

    let orient = |(a, b, c): (u32, u32, u32)| -> Option<(u32, u32, u32)> {
        let (pa, pb, pc) = (positions[a as usize], positions[b as usize], positions[c as usize]);
        let face = pb.sub(pa).cross(pc.sub(pa));
        if face.dot(face) < 1e-30 { return None; }
        let nsum = normals[a as usize].add(normals[b as usize]).add(normals[c as usize]);
        Some(if face.dot(nsum) < 0.0 { (a, c, b) } else { (a, b, c) })
    };
    let mut tris: Vec<(u32, u32, u32)> = strong.iter().filter_map(|&t| orient(t)).collect();
    fill_small_holes(&mut tris, 12);

    let mut triangles = Vec::with_capacity(tris.len());
    for &(a, b, c) in &tris {
        let Some((a, b, c)) = orient((a, b, c)) else { continue };
        let (ia, ib, ic) = (a as usize, b as usize, c as usize);
        triangles.push(Triangle { splat: false,
            vertices: [positions[ia], positions[ib], positions[ic]],
            normal: normals[ia].add(normals[ib]).add(normals[ic]).normalized(),
            color: if has_colors { Some(colors[ia]) } else { None },
            vertex_colors: if has_colors { Some([colors[ia], colors[ib], colors[ic]]) } else { None },
            group_id: None,
            alpha: None,
            vertex_normals: Some([normals[ia], normals[ib], normals[ic]]),
            smoothing_group: None,
            vertex_scalars: if has_scalars { Some([cloud.scalars[ia], cloud.scalars[ib], cloud.scalars[ic]]) } else { None },
            uvs: None,
            tex: None,
        });
    }
    triangles
}

/// Close small holes: cycles of at most `max_len` edges among the edges used
/// by exactly one triangle, each filled with a fan. Orientation is ignored
/// (neighbouring triangles across a crease may disagree); callers orient the
/// new triangles afterwards.
fn fill_small_holes(tris: &mut Vec<(u32, u32, u32)>, max_len: usize) {
    let key = |u: u32, v: u32| (u.min(v), u.max(v));
    let mut uses: HashMap<(u32, u32), u8, FxBuildHasher> =
        HashMap::with_capacity_and_hasher(tris.len() * 3, FxBuildHasher::default());
    for &(a, b, c) in tris.iter() {
        for (u, v) in [(a, b), (b, c), (c, a)] { *uses.entry(key(u, v)).or_insert(0) += 1; }
    }
    let mut border: Vec<(u32, u32)> = uses.iter().filter(|(_, &n)| n == 1).map(|(&e, _)| e).collect();
    border.sort_unstable();
    let mut adj: HashMap<u32, Vec<u32>, FxBuildHasher> = HashMap::with_hasher(FxBuildHasher::default());
    for &(u, v) in &border {
        adj.entry(u).or_default().push(v);
        adj.entry(v).or_default().push(u);
    }
    for vs in adj.values_mut() { vs.sort_unstable(); }
    let mut used: HashMap<(u32, u32), (), FxBuildHasher> = HashMap::with_hasher(FxBuildHasher::default());
    let mut parent: HashMap<u32, u32, FxBuildHasher> = HashMap::with_hasher(FxBuildHasher::default());
    let mut frontier: Vec<u32> = Vec::new();
    let mut next_frontier: Vec<u32> = Vec::new();
    let mut lp: Vec<u32> = Vec::with_capacity(max_len);
    for &(a, b) in &border {
        if used.contains_key(&(a, b)) { continue; }
        parent.clear();
        parent.insert(b, a);
        frontier.clear();
        frontier.push(b);
        let mut found = false;
        for _ in 1..max_len {
            next_frontier.clear();
            for &u in &frontier {
                for &v in &adj[&u] {
                    if (u == b && v == a) || used.contains_key(&key(u, v)) { continue; }
                    if v == a { parent.insert(a, u); found = true; break; }
                    if parent.contains_key(&v) { continue; }
                    parent.insert(v, u);
                    next_frontier.push(v);
                }
                if found { break; }
            }
            if found || next_frontier.is_empty() { break; }
            std::mem::swap(&mut frontier, &mut next_frontier);
        }
        if !found { continue; }
        lp.clear();
        let mut cur = a;
        loop {
            lp.push(cur);
            let p = parent[&cur];
            if p == a { break; }
            cur = p;
        }
        if lp.len() < 3 { continue; }
        for w in 0..lp.len() {
            used.insert(key(lp[w], lp[(w + 1) % lp.len()]), ());
        }
        for w in 1..lp.len() - 1 {
            tris.push((lp[0], lp[w], lp[w + 1]));
        }
    }
}

fn denoise_color(i: usize, nb: &Neighbours, colors: &[(u8, u8, u8)]) -> (u8, u8, u8) {
    let nbr = nb.of(i);
    if nbr.is_empty() { return colors[i]; }
    let mut best = colors[i];
    let mut best_cost = f64::INFINITY;
    let all = std::iter::once(i as u32).chain(nbr.iter().copied());
    for ci in all.clone().map(|j| colors[j as usize]) {
        let mut cost = 0.0;
        for cj in all.clone().map(|j| colors[j as usize]) {
            let (dr, dg, db) = (ci.0 as f64 - cj.0 as f64, ci.1 as f64 - cj.1 as f64, ci.2 as f64 - cj.2 as f64);
            cost += (dr * dr + dg * dg + db * db).sqrt();
        }
        if cost < best_cost { best_cost = cost; best = ci; }
    }
    best
}

const SPLAT_SIDES: usize = 8;
const SPLAT_RINGS: &[(f64, f64, Option<f32>)] = &[(0.0, 0.93, None), (0.93, 1.0, Some(0.5))];
const SPLAT_MIN_FACE: f64 = 0.34;

pub(crate) fn splats(cloud: &PointCloud, config: &RenderConfig, native: bool) -> Vec<Triangle> {
    let unique = dedup(cloud);
    let cloud = unique.as_ref().unwrap_or(cloud);
    let n = cloud.positions.len();
    if n == 0 { return Vec::new(); }
    let positions = &cloud.positions;
    let (bmin, bmax) = crate::render::bbox_of(positions.iter().copied());
    let diag = bmax.sub(bmin).length();
    if diag < 1e-12 { return Vec::new(); }

    let view = resolve_config_view(config, bbox_center(bmin, bmax), bbox_radius(bmin, bmax));
    let cam_dir = view.camera.sub(view.center).normalized();
    let (cam_right, cam_up) = cam_dir.tangent_basis();

    let has_normals = cloud.normals.len() == n;
    let adaptive = config.point_size <= 0.0;
    let nb = if adaptive || !has_normals { Some(neighbours(positions, bmin, bmax, 8)) } else { None };
    let radii: Vec<f64> = match (&nb, adaptive) {
        (Some(nb), true) => {
            let base = (diag / (n as f64).sqrt() * 0.5).fmax(diag * 1e-3);
            let mut r: Vec<f64> = (0..n)
                .map(|i| {
                    let ds = nb.dsq_of(i);
                    if ds.is_empty() { base } else { ((ds[3.min(ds.len() - 1)] as f64).sqrt() * 0.95).fmax(diag * 1e-4) }
                })
                .collect();
            let mut sorted = r.clone();
            let mid = n / 2;
            sorted.select_nth_unstable_by(mid, f64::total_cmp);
            let cap = sorted[mid] * 3.0;
            for v in &mut r { *v = v.fmin(cap); }
            r
        }
        _ => vec![config.point_size; n],
    };
    let normals: Option<Vec<Vec3>> = if has_normals {
        Some(cloud.normals.iter().map(|v| v.normalized()).collect())
    } else {
        nb.as_ref().map(|nb| {
            let mut nv: Vec<Vec3> = (0..n).map(|i| pca_normal(positions[i], nb.of(i), positions, view.camera)).collect();
            orient_normals(&mut nv, positions, nb, view.camera);
            nv
        })
    };

    let rim_dir: Vec<(f64, f64)> = (0..SPLAT_SIDES)
        .map(|k| {
            let a = std::f64::consts::TAU * (k as f64) / (SPLAT_SIDES as f64);
            (a.cos(), a.sin())
        })
        .collect();
    let has_colors = cloud.colors.len() == n;
    let flat_lit = has_colors && config.shading.is_empty();
    let rings: &[(f64, f64, Option<f32>)] = if flat_lit { &[(0.0, 1.0, None)] } else { SPLAT_RINGS };
    let per_point = if native { 1 } else { rings.iter().map(|r| if r.0 <= 0.0 { 1 } else { 2 }).sum::<usize>() * SPLAT_SIDES };
    let mut triangles = Vec::with_capacity(n * per_point);
    for i in 0..n {
        let p = positions[i];
        let r = radii[i];
        let color = if has_colors { Some(cloud.colors[i]) } else { None };
        let vertex_colors = color.map(|c| [c, c, c]);
        let nrm = normals.as_ref().map(|nv| nv[i]);
        let shade_n = nrm.unwrap_or(cam_dir);
        let (right, up) = match nrm {
            Some(dn) => {
                let f = dn.dot(cam_dir);
                let disc_n = if f.abs() >= SPLAT_MIN_FACE {
                    dn
                } else {
                    let s = if f < 0.0 { -1.0 } else { 1.0 };
                    let tang = dn.sub(cam_dir.scale(f)).normalized();
                    cam_dir.scale(s * SPLAT_MIN_FACE).add(tang.scale((1.0 - SPLAT_MIN_FACE * SPLAT_MIN_FACE).sqrt())).normalized()
                };
                disc_n.tangent_basis()
            }
            None => (cam_right, cam_up),
        };
        let vn = if flat_lit { None } else { Some([shade_n; 3]) };
        let mut push_tri = |v: [Vec3; 3], alpha: Option<f32>| {
            triangles.push(Triangle {
                splat: native,
                vertices: v,
                normal: cam_dir,
                color,
                vertex_colors,
                group_id: None,
                alpha,
                vertex_normals: vn,
                smoothing_group: None,
                vertex_scalars: None,
                uvs: None,
                tex: None,
            });
        };
        if native {
            push_tri([p, p.add(right.scale(r)), p.add(up.scale(r))], None);
            continue;
        }
        let at = |c: f64, s: f64, rr: f64| p.add(right.scale(r * rr * c)).add(up.scale(r * rr * s));
        for &(r0, r1, alpha) in rings {
            for k in 0..SPLAT_SIDES {
                let (c0, s0) = rim_dir[k];
                let (c1, s1) = rim_dir[(k + 1) % SPLAT_SIDES];
                let o0 = at(c0, s0, r1);
                let o1 = at(c1, s1, r1);
                if r0 <= 0.0 {
                    push_tri([p, o0, o1], alpha);
                } else {
                    let i0 = at(c0, s0, r0);
                    let i1 = at(c1, s1, r0);
                    push_tri([i0, o0, o1], alpha);
                    push_tri([i0, o1, i1], alpha);
                }
            }
        }
    }
    triangles
}
