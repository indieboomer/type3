//! Bounded, cancellable nested density-brick clipmaps. Geometry is disposable;
//! every level samples the milestone-3 planet field in planet-local f64 metres.
pub mod mesh;
use crate::{
    app::camera::Camera,
    planet::{Planet, PlanetField},
    renderer::sphere::Vertex,
    world::{CELL_SIZE_METERS, CellCoord, CellSample, MaterialId},
};
use glam::DVec3;
use mesh::{BRICK_CELLS, BrickMesh, mesh_brick};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::Instant,
};

pub const LEVELS: usize = 9;
pub const MAX_BRICKS: usize = LEVELS * 64;
pub const MAX_VERTICES: usize = 400_000;
pub const MAX_INDICES: usize = 2_400_000;
pub const ACTIVATION_ALTITUDE: f64 = 220.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub center: DVec3,
    pub half: f64,
}
impl Bounds {
    pub fn relative(self, camera: &Camera) -> [f32; 4] {
        let p = camera.relative(self.center);
        [p.x, p.y, p.z, self.half as f32]
    }
    pub fn weight(self, p: DVec3) -> f64 {
        let t = (((p - self.center).abs().max_element() / self.half - 0.75) / 0.20).clamp(0.0, 1.0);
        1.0 - t * t * (3.0 - 2.0 * t)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct BrickKey {
    level: usize,
    xyz: [i64; 3],
}
fn spacing(level: usize) -> f64 {
    CELL_SIZE_METERS * (1u64 << level) as f64
}
fn anchor(position: DVec3, level: usize) -> [i64; 3] {
    let p = (position / (spacing(level) * BRICK_CELLS as f64)).round();
    [p.x as i64, p.y as i64, p.z as i64]
}
fn bounds(position: DVec3, level: usize) -> Bounds {
    let a = anchor(position, level);
    let size = spacing(level) * BRICK_CELLS as f64;
    Bounds {
        center: DVec3::new(a[0] as f64, a[1] as f64, a[2] as f64) * size,
        half: size * 2.0,
    }
}

/// Render-only approximation at the outer boundary. Inner levels always use
/// canonical density. The outer mesh converges to the macro boundary before
/// its overlap with the distant renderer; it does not change cell queries.
struct RenderField {
    planet: Planet,
    outer: Option<Bounds>,
}
impl PlanetField for RenderField {
    fn sample_density(&self, p: DVec3) -> f64 {
        let full = self.planet.sample_density(p);
        if let Some(b) = self.outer {
            let t = (((p - b.center).abs().max_element() / b.half - 0.55) / 0.20).clamp(0.0, 1.0);
            let w = t * t * (3.0 - 2.0 * t);
            full * (1.0 - w) - self.planet.altitude(p) * w
        } else {
            full
        }
    }
    fn sample_material(&self, p: DVec3) -> MaterialId {
        // A coarse interpolated crossing can lie above the exact field. Use
        // the shared solid surface palette there, never paint terrain as air.
        let material = self.planet.sample_material(p);
        if matches!(material, MaterialId::Air | MaterialId::Vacuum) {
            let d = p.try_normalize().unwrap_or(DVec3::Y);
            self.planet
                .surface_material(d, self.planet.surface_normal(d))
        } else {
            material
        }
    }
    fn sample_cell(&self, cell: CellCoord) -> CellSample {
        self.planet.sample_cell(cell)
    }
}

pub struct VolumeLevel {
    pub bounds: Bounds,
    pub bricks: Vec<Arc<BrickMesh>>,
}
pub struct Clipmap {
    pub levels: Vec<VolumeLevel>,
    pub generation_ms: f64,
    pub samples: usize,
    pub bytes: usize,
    pub triangles: usize,
    pub source_position: DVec3,
}

/// Vulkan scene batches share one vertex/index upload; each batch has its own
/// nested overlap bounds in the same camera-relative coordinate system.
#[derive(Clone, Copy)]
pub struct DrawBatch {
    pub first_index: u32,
    pub index_count: u32,
    pub outer: [f32; 4],
    pub inner: [f32; 4],
    pub kind: f32, // 0 macro, 1 volume; stars are identified by zero normals
    pub strength: f32,
}
pub struct SceneDraw<'a> {
    pub matrix: [f32; 16],
    pub batches: &'a [DrawBatch],
}
impl DrawBatch {
    pub fn macro_only(indices: usize) -> Self {
        Self {
            first_index: 0,
            index_count: indices as u32,
            outer: [0.0; 4],
            inner: [0.0; 4],
            kind: 0.0,
            strength: 1.0,
        }
    }
}
impl Clipmap {
    pub fn append(
        &self,
        camera: &Camera,
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u32>,
        batches: &mut Vec<DrawBatch>,
        strength: f32,
    ) {
        batches[0].strength = strength;
        if let Some(last) = self.levels.last() {
            batches[0].inner = last.bounds.relative(camera);
        }
        for (i, level) in self.levels.iter().enumerate() {
            let start = indices.len();
            for brick in &level.bricks {
                let base = vertices.len() as u32;
                let origin = brick.origin - camera.position;
                vertices.extend(brick.vertices.iter().map(|v| {
                    Vertex {
                        position: (origin + DVec3::from_array(v.position.map(f64::from)))
                            .as_vec3()
                            .to_array(),
                        ..*v
                    }
                }));
                indices.extend(brick.indices.iter().map(|index| base + index));
            }
            batches.push(DrawBatch {
                first_index: start as u32,
                index_count: (indices.len() - start) as u32,
                outer: level.bounds.relative(camera),
                inner: if i == 0 {
                    [0.0; 4]
                } else {
                    self.levels[i - 1].bounds.relative(camera)
                },
                kind: 1.0,
                strength,
            });
        }
    }
}

type Cache = HashMap<BrickKey, Arc<BrickMesh>>;
fn build(
    planet: Planet,
    position: DVec3,
    cache: &mut Cache,
    cancelled: impl Fn() -> bool,
) -> Result<Option<Clipmap>, String> {
    let started = Instant::now();
    let mut active = HashSet::new();
    for level in 0..LEVELS {
        let a = anchor(position, level);
        for z in -2..2 {
            for y in -2..2 {
                for x in -2..2 {
                    active.insert(BrickKey {
                        level,
                        xyz: [a[0] + x, a[1] + y, a[2] + z],
                    });
                }
            }
        }
    }
    // Outermost bricks depend on that level's transition bounds. They cannot
    // be reused after its anchor moves; all other bricks are world-keyed.
    cache.retain(|k, _| active.contains(k) && k.level != LEVELS - 1);
    let mut result = Clipmap {
        levels: Vec::new(),
        generation_ms: 0.0,
        samples: 0,
        bytes: 0,
        triangles: 0,
        source_position: position,
    };
    let mut vertices = 0;
    let mut indices = 0;
    for level in 0..LEVELS {
        let b = bounds(position, level);
        let field = RenderField {
            planet,
            outer: (level == LEVELS - 1).then_some(b),
        };
        let a = anchor(position, level);
        let mut bricks = Vec::with_capacity(64);
        for z in -2..2 {
            for y in -2..2 {
                for x in -2..2 {
                    if cancelled() {
                        return Ok(None);
                    }
                    let key = BrickKey {
                        level,
                        xyz: [a[0] + x, a[1] + y, a[2] + z],
                    };
                    let part = cache
                        .entry(key)
                        .or_insert_with(|| {
                            Arc::new(mesh_brick(
                                &field,
                                key.xyz.map(|v| v * BRICK_CELLS),
                                spacing(level),
                            ))
                        })
                        .clone();
                    vertices += part.vertices.len();
                    indices += part.indices.len();
                    if vertices > MAX_VERTICES || indices > MAX_INDICES {
                        cache.clear();
                        return Err("Volumetric mesh exceeded its bounded GPU budget".into());
                    }
                    result.samples += part.samples;
                    result.bytes += part.bytes();
                    result.triangles += part.indices.len() / 3;
                    bricks.push(part);
                }
            }
        }
        result.levels.push(VolumeLevel { bounds: b, bricks });
    }
    result.generation_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(Some(result))
}

struct Request {
    id: u64,
    position: Option<DVec3>,
}
type MeshResult = Result<Option<Clipmap>, String>;
struct Shared {
    request: Mutex<Option<Request>>,
    result: Mutex<Option<(u64, MeshResult)>>,
    wake: Condvar,
    revision: AtomicU64,
    stop: AtomicBool,
    cached_bricks: AtomicUsize,
}
pub struct VolumeWorker {
    shared: Arc<Shared>,
    thread: Option<thread::JoinHandle<()>>,
    last_anchor: Option<[i64; 3]>,
    pub busy: bool,
    pub error: Option<String>,
}
impl VolumeWorker {
    pub fn new(planet: Planet) -> Self {
        let shared = Arc::new(Shared {
            request: Mutex::new(None),
            result: Mutex::new(None),
            wake: Condvar::new(),
            revision: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            cached_bricks: AtomicUsize::new(0),
        });
        let worker = shared.clone();
        let thread = thread::Builder::new()
            .name("volumetric-terrain".into())
            .spawn(move || {
                let mut cache = Cache::new();
                loop {
                    let request = {
                        let mut slot = worker.request.lock().unwrap();
                        while slot.is_none() && !worker.stop.load(Ordering::Relaxed) {
                            slot = worker.wake.wait(slot).unwrap();
                        }
                        if worker.stop.load(Ordering::Relaxed) {
                            break;
                        }
                        slot.take().unwrap()
                    };
                    let cancelled = || {
                        worker.stop.load(Ordering::Relaxed)
                            || worker.revision.load(Ordering::Relaxed) != request.id
                    };
                    let result = if let Some(position) = request.position {
                        build(planet, position, &mut cache, cancelled)
                    } else {
                        cache.clear();
                        Ok(None)
                    };
                    worker.cached_bricks.store(cache.len(), Ordering::Relaxed);
                    if !cancelled() {
                        *worker.result.lock().unwrap() = Some((request.id, result));
                    }
                }
            })
            .expect("volume worker");
        Self {
            shared,
            thread: Some(thread),
            last_anchor: None,
            busy: false,
            error: None,
        }
    }

    /// Latest request replaces an obsolete one. At most one pending request,
    /// one completed mesh and MAX_BRICKS cached bricks can exist.
    pub fn update(&mut self, position: Option<DVec3>, current: &mut Option<Clipmap>) {
        let next_anchor = position.map(|p| anchor(p, 0));
        if next_anchor != self.last_anchor {
            self.last_anchor = next_anchor;
            let id = self.shared.revision.fetch_add(1, Ordering::Relaxed) + 1;
            *self.shared.request.lock().unwrap() = Some(Request { id, position });
            self.shared.wake.notify_one();
            self.busy = true;
            if position.is_none() {
                *current = None;
            }
        }
        if let Some((id, result)) = self.shared.result.lock().unwrap().take()
            && id == self.shared.revision.load(Ordering::Relaxed)
        {
            self.busy = false;
            match result {
                Ok(mesh) => {
                    *current = mesh;
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
    }

    pub fn cached_bricks(&self) -> usize {
        self.shared.cached_bricks.load(Ordering::Relaxed)
    }
}
impl Drop for VolumeWorker {
    fn drop(&mut self) {
        // Hold the condition mutex while signalling to avoid a lost shutdown wake.
        let _guard = self.shared.request.lock().unwrap();
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.wake.notify_one();
        drop(_guard);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipmaps_are_nested_world_aligned_and_blend_partitions_unity() {
        for position in [
            DVec3::new(6371000.3, 1.7, -2.1),
            DVec3::new(-6371000.3, -1.7, 2.1),
        ] {
            for level in 0..LEVELS - 1 {
                let a = bounds(position, level);
                let b = bounds(position, level + 1);
                assert!((a.center - b.center).abs().max_element() + a.half <= b.half + 1e-6);
            }
            for i in 0..1000 {
                let p = position + DVec3::X * i as f64 * 0.5;
                let mut previous = 0.0;
                let mut sum = 0.0;
                for level in 0..LEVELS {
                    let weight = bounds(position, level).weight(p);
                    assert!(weight + 1e-9 >= previous);
                    sum += weight - previous;
                    previous = weight;
                }
                sum += 1.0 - previous;
                assert!((sum - 1.0).abs() < 1e-9);
            }
        }
        assert_eq!(spacing(0), 0.1);
    }

    #[test]
    fn worker_retargets_generates_fine_surface_and_evicts_on_ascent() {
        let planet = Planet {
            radius: 6371000.0,
            seed: 928371,
        };
        let mut worker = VolumeWorker::new(planet);
        let mut mesh = None;
        let initial = DVec3::X * (planet.outer_surface_radius(DVec3::X) + 0.5);
        let destination = -DVec3::X * (planet.outer_surface_radius(-DVec3::X) + 0.5);
        worker.update(Some(initial), &mut mesh);
        worker.update(Some(destination), &mut mesh);
        let started = Instant::now();
        while worker.busy {
            assert!(
                started.elapsed().as_secs() < 15,
                "worker did not complete: {:?}",
                worker.error
            );
            thread::sleep(std::time::Duration::from_millis(2));
            worker.update(Some(destination), &mut mesh);
        }
        assert!(worker.error.is_none(), "{:?}", worker.error);
        let result = mesh.as_ref().unwrap();
        assert_eq!(result.source_position, destination);
        assert!(
            result.levels[0]
                .bricks
                .iter()
                .any(|b| !b.indices.is_empty())
        );
        assert!(result.triangles * 3 <= MAX_INDICES);
        assert!(worker.cached_bricks() <= MAX_BRICKS);
        worker.update(None, &mut mesh);
        assert!(mesh.is_none());
        while worker.busy {
            assert!(started.elapsed().as_secs() < 15);
            thread::sleep(std::time::Duration::from_millis(2));
            worker.update(None, &mut mesh);
        }
        assert_eq!(worker.cached_bricks(), 0);
    }
}
