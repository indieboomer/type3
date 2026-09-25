//! Temporary fixed-resolution sphere for milestone 0, not procedural terrain.
use crate::app::camera::Camera;
use glam::DVec3;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

pub struct Sphere {
    pub positions: Vec<DVec3>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

impl Sphere {
    pub fn new(radius: f64, resolution: u32) -> Self {
        assert!(resolution > 0);
        let mut mesh = Self {
            positions: Vec::new(),
            normals: Vec::new(),
            indices: Vec::new(),
        };
        for face in 0..6 {
            let start = mesh.positions.len() as u32;
            for y in 0..=resolution {
                for x in 0..=resolution {
                    let u = 2.0 * x as f64 / resolution as f64 - 1.0;
                    let v = 2.0 * y as f64 / resolution as f64 - 1.0;
                    let direction = cube_direction(face, u, v);
                    mesh.positions.push(direction * radius);
                    mesh.normals.push(direction.as_vec3().to_array());
                }
            }
            for y in 0..resolution {
                for x in 0..resolution {
                    let a = start + y * (resolution + 1) + x;
                    let b = a + resolution + 1;
                    mesh.indices
                        .extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
                }
            }
        }
        mesh
    }

    pub fn write_relative_vertices(&self, camera: &Camera, output: &mut Vec<Vertex>) {
        output.clear();
        output.extend(
            self.positions
                .iter()
                .zip(&self.normals)
                .map(|(&p, &normal)| Vertex {
                    position: camera.relative(p).to_array(),
                    normal,
                }),
        );
    }
}

pub fn cube_direction(face: usize, u: f64, v: f64) -> DVec3 {
    match face {
        0 => DVec3::new(1.0, v, -u),
        1 => DVec3::new(-1.0, v, u),
        2 => DVec3::new(u, 1.0, -v),
        3 => DVec3::new(u, -1.0, v),
        4 => DVec3::new(u, v, 1.0),
        5 => DVec3::new(-u, v, -1.0),
        _ => panic!("invalid cube face"),
    }
    .normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cube_edges_match_exactly() {
        for i in 0..=32 {
            let t = i as f64 / 16.0 - 1.0;
            assert_eq!(cube_direction(0, -1.0, t), cube_direction(4, 1.0, t));
            assert_eq!(cube_direction(0, 1.0, t), cube_direction(5, -1.0, t));
            assert_eq!(cube_direction(2, t, -1.0), cube_direction(4, t, 1.0));
        }
        let mesh = Sphere::new(6_371_000.0, 8);
        assert!(
            mesh.positions
                .iter()
                .all(|p| (p.length() - 6_371_000.0).abs() < 1e-8)
        );
        assert!(
            mesh.indices
                .iter()
                .all(|&i| (i as usize) < mesh.positions.len())
        );
    }
}
