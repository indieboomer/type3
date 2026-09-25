//! Consistent six-tetrahedron decomposition of each sampled cube. Unlike a
//! height mesh this extracts every sign crossing, including cavity walls.
use crate::{planet::PlanetField, renderer::sphere::Vertex};
use glam::DVec3;
use std::collections::HashMap;

pub const BRICK_CELLS: i64 = 8;
const SIDE: usize = BRICK_CELLS as usize + 1;

#[derive(Default)]
pub struct BrickMesh {
    pub origin: DVec3,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub samples: usize,
}

impl BrickMesh {
    pub fn bytes(&self) -> usize {
        self.vertices.len() * size_of::<Vertex>() + self.indices.len() * size_of::<u32>()
    }
}

pub fn mesh_brick(field: &impl PlanetField, lattice_origin: [i64; 3], spacing: f64) -> BrickMesh {
    let origin = DVec3::new(
        lattice_origin[0] as f64,
        lattice_origin[1] as f64,
        lattice_origin[2] as f64,
    ) * spacing;
    let mut mesh = BrickMesh {
        origin,
        ..Default::default()
    };
    let mut positions = Vec::with_capacity(SIDE * SIDE * SIDE);
    let mut density = Vec::with_capacity(SIDE * SIDE * SIDE);
    for z in 0..SIDE {
        for y in 0..SIDE {
            for x in 0..SIDE {
                // Integer lattice address first: shared brick corners evaluate the
                // identical f64 position, even on the negative side of the planet.
                let p = DVec3::new(
                    (lattice_origin[0] + x as i64) as f64,
                    (lattice_origin[1] + y as i64) as f64,
                    (lattice_origin[2] + z as i64) as f64,
                ) * spacing;
                positions.push(p);
                density.push(field.sample_density(p));
            }
        }
    }
    mesh.samples = density.len();
    if density.iter().all(|&d| d > 0.0) || density.iter().all(|&d| d <= 0.0) {
        return mesh;
    }
    let mut edges = HashMap::<(usize, usize), u32>::new();
    let index = |x, y, z| (z * SIDE + y) * SIDE + x;
    const TETS: [[usize; 4]; 6] = [
        [0, 1, 3, 7],
        [0, 3, 2, 7],
        [0, 2, 6, 7],
        [0, 6, 4, 7],
        [0, 4, 5, 7],
        [0, 5, 1, 7],
    ];
    for z in 0..SIDE - 1 {
        for y in 0..SIDE - 1 {
            for x in 0..SIDE - 1 {
                let corners: [usize; 8] = std::array::from_fn(|i| {
                    index(x + (i & 1), y + ((i >> 1) & 1), z + ((i >> 2) & 1))
                });
                if corners.iter().all(|&i| density[i] > 0.0)
                    || corners.iter().all(|&i| density[i] <= 0.0)
                {
                    continue;
                }
                for tet in TETS {
                    let mut inside = Vec::with_capacity(4);
                    let mut outside = Vec::with_capacity(4);
                    for i in tet {
                        let i = corners[i];
                        if density[i] > 0.0 {
                            inside.push(i);
                        } else {
                            outside.push(i);
                        }
                    }
                    if inside.is_empty() || outside.is_empty() {
                        continue;
                    }
                    let mut vertex = |a: usize, b: usize| -> u32 {
                        let (a, b) = if a < b { (a, b) } else { (b, a) };
                        *edges.entry((a, b)).or_insert_with(|| {
                            let (mut pa, mut pb) = (positions[a], positions[b]);
                            let (mut da, mut db) = (density[a], density[b]);
                            let mut p = pa.lerp(pb, (da / (da - db)).clamp(0.0, 1.0));
                            // Density is not an exact SDF or linear along an
                            // edge. Refine within its sign bracket so a cavity
                            // wall cannot acquire the opposite wall's normal.
                            for _ in 0..8 {
                                let value = field.sample_density(p);
                                if value.abs() < spacing * 0.0001 {
                                    break;
                                }
                                if (value > 0.0) == (da > 0.0) {
                                    pa = p;
                                    da = value;
                                } else {
                                    pb = p;
                                    db = value;
                                }
                                p = pa.lerp(pb, (da / (da - db)).clamp(0.1, 0.9));
                            }
                            let epsilon = 0.025_f64.min(spacing * 0.25);
                            let derivative = |axis| {
                                field.sample_density(p + axis * epsilon)
                                    - field.sample_density(p - axis * epsilon)
                            };
                            let normal = -DVec3::new(
                                derivative(DVec3::X),
                                derivative(DVec3::Y),
                                derivative(DVec3::Z),
                            )
                            .try_normalize()
                            .unwrap_or(DVec3::Y);
                            let color = field
                                .sample_material(p - normal * (spacing * 0.05).min(0.05))
                                .color();
                            let index = mesh.vertices.len() as u32;
                            mesh.vertices.push(Vertex {
                                position: (p - origin).as_vec3().to_array(),
                                normal: normal.as_vec3().to_array(),
                                color,
                            });
                            index
                        })
                    };
                    let triangles = if inside.len() == 1 {
                        vec![[
                            vertex(inside[0], outside[0]),
                            vertex(inside[0], outside[1]),
                            vertex(inside[0], outside[2]),
                        ]]
                    } else if outside.len() == 1 {
                        vec![[
                            vertex(outside[0], inside[0]),
                            vertex(outside[0], inside[1]),
                            vertex(outside[0], inside[2]),
                        ]]
                    } else {
                        let a = vertex(inside[0], outside[0]);
                        let b = vertex(inside[0], outside[1]);
                        let c = vertex(inside[1], outside[0]);
                        let d = vertex(inside[1], outside[1]);
                        vec![[a, b, c], [b, d, c]]
                    };
                    for mut triangle in triangles {
                        let p = triangle.map(|i| {
                            DVec3::from_array(mesh.vertices[i as usize].position.map(f64::from))
                        });
                        let cross = (p[1] - p[0]).cross(p[2] - p[0]);
                        if cross.length_squared() < 1e-24 {
                            continue;
                        }
                        let normal = DVec3::from_array(
                            mesh.vertices[triangle[0] as usize].normal.map(f64::from),
                        );
                        if cross.dot(normal) < 0.0 {
                            triangle.swap(1, 2);
                        }
                        mesh.indices.extend_from_slice(&triangle);
                    }
                }
            }
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{CellCoord, CellSample, MaterialId};
    struct Plane;
    impl PlanetField for Plane {
        fn sample_density(&self, p: DVec3) -> f64 {
            0.337 - p.y
        }
        fn sample_material(&self, _: DVec3) -> MaterialId {
            MaterialId::Granite
        }
        fn sample_cell(&self, _: CellCoord) -> CellSample {
            unreachable!()
        }
    }
    #[test]
    fn plane_crossings_winding_and_shared_brick_edges() {
        let a = mesh_brick(&Plane, [-8, 0, 0], 0.1);
        let b = mesh_brick(&Plane, [0, 0, 0], 0.1);
        assert!(!a.indices.is_empty());
        for mesh in [&a, &b] {
            for v in &mesh.vertices {
                assert!((v.position[1] - 0.337).abs() < 1e-6);
                assert!(v.normal[1] > 0.999);
            }
            for tri in mesh.indices.as_chunks::<3>().0 {
                let p: Vec<_> = tri
                    .iter()
                    .map(|&i| glam::Vec3::from(mesh.vertices[i as usize].position))
                    .collect();
                assert!((p[1] - p[0]).cross(p[2] - p[0]).y > 0.0);
            }
        }
        let edge = |mesh: &BrickMesh| {
            let mut result: Vec<_> = mesh
                .vertices
                .iter()
                .filter_map(|v| {
                    let p = mesh.origin + DVec3::from_array(v.position.map(f64::from));
                    (p.x.abs() < 1e-6)
                        .then_some(((p.y * 1e6).round() as i64, (p.z * 1e6).round() as i64))
                })
                .collect();
            result.sort_unstable();
            result.dedup();
            result
        };
        assert_eq!(edge(&a), edge(&b));
        assert!(mesh_brick(&Plane, [0, 16, 0], 0.1).indices.is_empty());
    }

    struct HollowSphere;
    impl PlanetField for HollowSphere {
        fn sample_density(&self, p: DVec3) -> f64 {
            let r = p.distance(DVec3::new(0.417, 0.393, 0.421));
            (0.34 - r).min(r - 0.17)
        }
        fn sample_material(&self, _: DVec3) -> MaterialId {
            MaterialId::Limestone
        }
        fn sample_cell(&self, _: CellCoord) -> CellSample {
            unreachable!()
        }
    }
    #[test]
    fn closed_cavity_has_both_inner_and_outer_surfaces_and_no_open_edges() {
        let mesh = mesh_brick(&HollowSphere, [0, 0, 0], 0.1);
        let center = DVec3::new(0.417, 0.393, 0.421);
        let mut inner = 0;
        let mut outer = 0;
        for vertex in &mesh.vertices {
            let radial = DVec3::from_array(vertex.position.map(f64::from)) - center;
            let normal = DVec3::from_array(vertex.normal.map(f64::from));
            if radial.length() < 0.24 {
                inner += 1;
                assert!(normal.dot(radial) < 0.0);
            } else {
                outer += 1;
                assert!(normal.dot(radial) > 0.0);
            }
        }
        assert!(inner > 0 && outer > 0);
        let mut edges = HashMap::new();
        for &[a, b, c] in mesh.indices.as_chunks::<3>().0 {
            for (a, b) in [(a, b), (b, c), (c, a)] {
                *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        assert!(edges.values().all(|&count| count == 2));
    }
}
