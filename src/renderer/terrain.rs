//! Bounded cube-sphere quadtree. All patch samples use the same macro field.
use super::sphere::{Vertex, cube_direction};
use crate::{
    app::camera::Camera,
    planet::{Planet, hash},
};
use glam::DVec3;
use std::{
    collections::{HashMap, HashSet},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::Instant,
};

const GRID: u32 = 16;
pub const MAX_PATCHES: usize = 2048;
const MAX_DEPTH: u8 = 24;
const STAR_COUNT: usize = 2200;
pub const MAX_VERTICES: usize = MAX_PATCHES * (17 * 17 + 4 * 17) + STAR_COUNT * 4;
pub const MAX_INDICES: usize = MAX_PATCHES * (16 * 16 * 6 + 4 * 16 * 6) + STAR_COUNT * 6;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Patch {
    pub face: usize,
    pub depth: u8,
    pub x: u32,
    pub y: u32,
}

impl Patch {
    fn uv(self, x: f64, y: f64) -> (f64, f64) {
        let scale = 2.0 / (1u32 << self.depth) as f64;
        (
            -1.0 + (self.x as f64 + x) * scale,
            -1.0 + (self.y as f64 + y) * scale,
        )
    }
    fn direction(self, x: f64, y: f64) -> DVec3 {
        let (u, v) = self.uv(x, y);
        cube_direction(self.face, u, v)
    }
    fn children(self) -> [Self; 4] {
        std::array::from_fn(|i| Self {
            face: self.face,
            depth: self.depth + 1,
            x: self.x * 2 + i as u32 % 2,
            y: self.y * 2 + i as u32 / 2,
        })
    }
}

fn face_uv(d: DVec3) -> (usize, f64, f64) {
    let a = d.abs();
    if a.x >= a.y && a.x >= a.z {
        if d.x > 0.0 {
            (0, -d.z / a.x, d.y / a.x)
        } else {
            (1, d.z / a.x, d.y / a.x)
        }
    } else if a.y >= a.z {
        if d.y > 0.0 {
            (2, d.x / a.y, -d.z / a.y)
        } else {
            (3, d.x / a.y, d.z / a.y)
        }
    } else if d.z > 0.0 {
        (4, d.x / a.z, d.y / a.z)
    } else {
        (5, -d.x / a.z, d.y / a.z)
    }
}

fn neighbor(leaves: &HashSet<Patch>, direction: DVec3) -> Patch {
    let (face, u, v) = face_uv(direction);
    for depth in 0..=MAX_DEPTH {
        let n = 1u32 << depth;
        let p = Patch {
            face,
            depth,
            x: (((u + 1.0) * 0.5 * n as f64) as u32).min(n - 1),
            y: (((v + 1.0) * 0.5 * n as f64) as u32).min(n - 1),
        };
        if leaves.contains(&p) {
            return p;
        }
    }
    unreachable!("quadtree covers all six faces")
}

fn adjacent(p: Patch) -> [DVec3; 4] {
    [
        p.direction(-1e-5, 0.5),
        p.direction(1.00001, 0.5),
        p.direction(0.5, -1e-5),
        p.direction(0.5, 1.00001),
    ]
}

fn select(planet: Planet, camera: DVec3, pixels: f64) -> Vec<Patch> {
    let mut threshold = 24.0;
    loop {
        let mut stack: Vec<_> = (0..6)
            .map(|face| Patch {
                face,
                depth: 0,
                x: 0,
                y: 0,
            })
            .collect();
        let mut leaves = HashSet::new();
        while let Some(p) = stack.pop() {
            let width = planet.radius * 2.0 / (1u32 << p.depth) as f64;
            let center = planet.position(p.direction(0.5, 0.5));
            let distance = (camera.distance(center) - width * 0.75).max(0.1);
            let projected_spacing = width / GRID as f64 / distance * pixels * 0.866;
            if p.depth < MAX_DEPTH && projected_spacing > threshold {
                stack.extend(p.children());
            } else {
                leaves.insert(p);
            }
            if leaves.len() + stack.len() > MAX_PATCHES {
                break;
            }
        }
        if !stack.is_empty() || leaves.len() > MAX_PATCHES {
            threshold *= 1.4;
            continue;
        }
        // Query from each fine edge into its neighbor, including across cube faces.
        // Iterating to a fixed point enforces a maximum one-level difference.
        loop {
            let mut split = HashSet::new();
            for &p in &leaves {
                for d in adjacent(p) {
                    let q = neighbor(&leaves, d);
                    if p.depth > q.depth + 1 {
                        split.insert(q);
                    }
                }
            }
            if split.is_empty() {
                break;
            }
            if leaves.len() + split.len() * 3 > MAX_PATCHES {
                break;
            }
            for p in split {
                leaves.remove(&p);
                leaves.extend(p.children());
            }
        }
        if leaves.iter().any(|&p| {
            adjacent(p)
                .iter()
                .any(|&d| p.depth > neighbor(&leaves, d).depth + 1)
        }) {
            threshold *= 1.4;
            continue;
        }
        let mut result: Vec<_> = leaves.into_iter().collect();
        result.sort_unstable();
        return result;
    }
}

#[derive(Default)]
pub struct TerrainMesh {
    pub source_position: DVec3,
    pub positions: Vec<DVec3>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub patches: usize,
    pub depth: u8,
    pub generation_ms: f64,
}

fn make_patch(planet: Planet, patch: Patch) -> TerrainMesh {
    let mut mesh = TerrainMesh::default();
    for y in 0..=GRID {
        for x in 0..=GRID {
            let d = patch.direction(x as f64 / GRID as f64, y as f64 / GRID as f64);
            mesh.positions.push(planet.position(d));
            let (normal, color) = planet.appearance(d);
            mesh.normals.push(normal);
            mesh.colors.push(color);
        }
    }
    for y in 0..GRID {
        for x in 0..GRID {
            let a = y * (GRID + 1) + x;
            let b = a + GRID + 1;
            mesh.indices
                .extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    let width = planet.radius * 2.0 / (1u32 << patch.depth) as f64;
    // Radial skirts overlap the coarse edge's chord and elevation interpolation.
    let skirt = (width * 0.025 + width * width / planet.radius / 64.0).max(0.05);
    for edge in 0..4 {
        let start = mesh.positions.len() as u32;
        let edge_index = |i| match edge {
            0 => i,
            1 => GRID * (GRID + 1) + i,
            2 => i * (GRID + 1),
            _ => i * (GRID + 1) + GRID,
        };
        for i in 0..=GRID {
            let source = edge_index(i) as usize;
            let p = mesh.positions[source];
            mesh.positions.push(p - p.normalize() * skirt);
            mesh.normals.push(mesh.normals[source]);
            mesh.colors.push(mesh.colors[source]);
        }
        for i in 0..GRID {
            let a = edge_index(i);
            let b = edge_index(i + 1);
            mesh.indices
                .extend_from_slice(&[a, b, start + i, b, start + i + 1, start + i]);
        }
    }
    mesh
}

impl TerrainMesh {
    pub fn write_relative_vertices(&self, camera: &Camera, output: &mut Vec<Vertex>) {
        output.clear();
        output.extend(self.positions.iter().enumerate().map(|(i, &p)| Vertex {
            position: camera.relative(p).to_array(),
            normal: self.normals[i],
            color: self.colors[i],
        }));
        // Emissive quads on the celestial sphere: rotation only, no translation/parallax.
        for i in 0..STAR_COUNT {
            let random = |salt| hash(i as u64 + salt) as f64 / u64::MAX as f64;
            let y = random(9981) * 2.0 - 1.0;
            let angle = random(31871) * std::f64::consts::TAU;
            let r = (1.0 - y * y).sqrt();
            let d = DVec3::new(r * angle.cos(), y, r * angle.sin());
            let right = d
                .cross(if y.abs() < 0.99 { DVec3::Y } else { DVec3::X })
                .normalize();
            let up = right.cross(d);
            let size = 0.00035 + random(76123).powi(8) * 0.0010;
            let brightness = (0.28 + random(5611).powi(3) * 0.72) as f32;
            let color = [
                brightness,
                brightness * 0.93,
                brightness * (0.8 + random(691) as f32 * 0.2),
            ];
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                output.push(Vertex {
                    position: (d + (right * x + up * y) * size).as_vec3().to_array(),
                    normal: [0.0; 3],
                    color,
                });
            }
        }
    }
}

fn build(
    planet: Planet,
    camera: DVec3,
    pixels: f64,
    cache: &mut HashMap<Patch, TerrainMesh>,
) -> TerrainMesh {
    let start = Instant::now();
    let leaves = select(planet, camera, pixels);
    let active: HashSet<_> = leaves.iter().copied().collect();
    cache.retain(|p, _| active.contains(p));
    let mut mesh = TerrainMesh {
        source_position: camera,
        patches: leaves.len(),
        depth: leaves.iter().map(|p| p.depth).max().unwrap_or(0),
        ..Default::default()
    };
    for p in leaves {
        let part = cache.entry(p).or_insert_with(|| make_patch(planet, p));
        let base = mesh.positions.len() as u32;
        mesh.positions.extend_from_slice(&part.positions);
        mesh.normals.extend_from_slice(&part.normals);
        mesh.colors.extend_from_slice(&part.colors);
        mesh.indices.extend(part.indices.iter().map(|i| i + base));
    }
    let base = mesh.positions.len() as u32;
    for i in 0..STAR_COUNT as u32 {
        let a = base + i * 4;
        mesh.indices
            .extend_from_slice(&[a, a + 1, a + 2, a + 1, a + 3, a + 2]);
    }
    mesh.generation_ms = start.elapsed().as_secs_f64() * 1000.0;
    mesh
}

pub struct TerrainWorker {
    requests: SyncSender<(DVec3, f64)>,
    results: Receiver<TerrainMesh>,
    pub busy: bool,
    last_request: DVec3,
    last_pixels: f64,
}

impl TerrainWorker {
    pub fn new(planet: Planet, position: DVec3, pixels: f64) -> (Self, TerrainMesh) {
        let mut cache = HashMap::new();
        let mesh = build(planet, position, pixels, &mut cache);
        let (requests, incoming) = mpsc::sync_channel::<(DVec3, f64)>(1);
        let (outgoing, results) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("macro-terrain".into())
            .spawn(move || {
                while let Ok((position, pixels)) = incoming.recv() {
                    let mesh = build(planet, position, pixels, &mut cache);
                    if outgoing.send(mesh).is_err() {
                        break;
                    }
                }
            })
            .expect("terrain worker");
        (
            Self {
                requests,
                results,
                busy: false,
                last_request: position,
                last_pixels: pixels,
            },
            mesh,
        )
    }

    pub fn update(&mut self, planet: Planet, position: DVec3, pixels: f64) -> Option<TerrainMesh> {
        let result = self.results.try_recv().ok();
        if result.is_some() {
            self.busy = false;
        }
        let threshold = (planet.altitude(position).abs() * 0.12).max(0.2);
        if !self.busy
            && (position.distance(self.last_request) > threshold || pixels != self.last_pixels)
            && self.requests.try_send((position, pixels)).is_ok()
        {
            self.last_request = position;
            self.last_pixels = pixels;
            self.busy = true;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_balanced_lod_at_surface_poles_and_face_edges() {
        let planet = Planet {
            radius: 6_371_000.0,
            seed: 928371,
        };
        for d in [
            DVec3::X,
            -DVec3::X,
            DVec3::Y,
            DVec3::new(1.0, 1.0, 1.0).normalize(),
            DVec3::new(1.0, 0.0, 1.0).normalize(),
        ] {
            for altitude in [0.5, 1000.0, planet.radius * 2.0] {
                let leaves = select(planet, planet.position(d) + d * altitude, 800.0);
                assert!(leaves.len() <= MAX_PATCHES);
                let set: HashSet<_> = leaves.iter().copied().collect();
                let area: f64 = leaves.iter().map(|p| 4.0_f64.powi(-(p.depth as i32))).sum();
                assert!((area - 6.0).abs() < 1e-10);
                for p in leaves {
                    for d in adjacent(p) {
                        assert!(p.depth.abs_diff(neighbor(&set, d).depth) <= 1);
                    }
                }
            }
        }
    }

    #[test]
    fn shared_edges_and_skirts_have_valid_geometry() {
        let planet = Planet {
            radius: 6_371_000.0,
            seed: 12,
        };
        let a = make_patch(
            planet,
            Patch {
                face: 0,
                depth: 0,
                x: 0,
                y: 0,
            },
        );
        let b = make_patch(
            planet,
            Patch {
                face: 4,
                depth: 0,
                x: 0,
                y: 0,
            },
        );
        for y in 0..=GRID as usize {
            assert_eq!(a.positions[y * 17], b.positions[y * 17 + 16]);
        }
        assert_eq!(a.positions.len(), 357);
        assert!(a.indices.iter().all(|&i| (i as usize) < a.positions.len()));
        assert!(a.positions.iter().all(|p| p.is_finite()));
        assert!(a.positions[289].length() < a.positions[0].length());
    }

    #[test]
    fn star_sky_is_translation_invariant_and_responds_to_rotation() {
        let mesh = TerrainMesh::default();
        let mut camera = Camera::new(6_371_000.0);
        let mut a = Vec::new();
        let mut b = Vec::new();
        mesh.write_relative_vertices(&camera, &mut a);
        camera.position = DVec3::new(-6_371_001.0, 123.0, 900.0);
        mesh.write_relative_vertices(&camera, &mut b);
        assert_eq!(
            bytemuck::cast_slice::<Vertex, u8>(&a),
            bytemuck::cast_slice::<Vertex, u8>(&b)
        );
        assert_eq!(a.len(), STAR_COUNT * 4);
        let direction = glam::Vec3::from(a[0].position);
        let before = camera.view_projection(1.6) * direction.extend(1.0);
        camera.mode = crate::app::camera::CameraMode::FreeFlight;
        camera.rotate(100.0, 40.0);
        let after = camera.view_projection(1.6) * direction.extend(1.0);
        assert!(before.distance(after) > 0.01);
    }

    #[test]
    fn skirts_overlap_coarse_chords_at_mixed_lod_edges() {
        let planet = Planet {
            radius: 6_371_000.0,
            seed: 928371,
        };
        for face in 0..6 {
            for depth in [0, 3, 8, 14, 22] {
                let p = Patch {
                    face,
                    depth,
                    x: (1 << depth) / 3,
                    y: (1 << depth) / 2,
                };
                let mesh = make_patch(planet, p);
                for i in 0..GRID as usize {
                    let a = mesh.positions[i];
                    let b = mesh.positions[i + 1];
                    let d = p.direction((i as f64 + 0.5) / GRID as f64, 0.0);
                    let fine = planet.position(d);
                    let chord = a.lerp(b, 0.5).length();
                    let skirt = a.length() - mesh.positions[289 + i].length();
                    assert!(
                        (fine.length() - chord).abs() < skirt,
                        "face {face}, depth {depth}, gap {} > skirt {skirt}",
                        fine.length() - chord
                    );
                }
            }
        }
    }
}
