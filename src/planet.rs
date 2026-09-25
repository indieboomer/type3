//! Seeded planet: inexpensive macro approximation and canonical 3D matter field.
pub mod field;
use crate::world::{GenerationVersion, MaterialId};
pub use field::PlanetField;
use glam::DVec3;

pub const GENERATION_VERSION: GenerationVersion = GenerationVersion(2);
// Preserve the milestone-2 macro landscape while version 2 adds matter detail.
const MACRO_VERSION: u32 = 1;

#[derive(Clone, Copy)]
pub struct Planet {
    pub radius: f64,
    pub seed: u64,
}

pub fn hash(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

fn noise(p: DVec3, seed: u64) -> f64 {
    let base = p.floor();
    let f = p - base;
    let t = f * f * (DVec3::splat(3.0) - 2.0 * f);
    let mut value = 0.0;
    for z in 0..2 {
        for y in 0..2 {
            for x in 0..2 {
                let h = hash(
                    seed ^ hash((base.x as i64 + x) as u64)
                        ^ hash((base.y as i64 + y) as u64).rotate_left(21)
                        ^ hash((base.z as i64 + z) as u64).rotate_left(42),
                );
                let weight = |i, a| if i == 0 { 1.0 - a } else { a };
                value += (h as f64 / u64::MAX as f64 * 2.0 - 1.0)
                    * weight(x, t.x)
                    * weight(y, t.y)
                    * weight(z, t.z);
            }
        }
    }
    value
}

impl Planet {
    pub fn elevation(&self, d: DVec3) -> f64 {
        let seed = self.seed ^ hash(MACRO_VERSION as u64);
        let continent = noise(d * 2.6 + DVec3::splat(7.3), seed);
        let ridge = 1.0 - noise(d * 22.0, seed ^ 31).abs();
        let mountains = ridge.powi(5) * (continent * 4.0 + 0.3).clamp(0.0, 1.0);
        let regional = noise(d * 110.0, seed ^ 71) * 0.00009
            + noise(d * 440.0, seed ^ 101) * 0.000022
            + noise(d * 1760.0, seed ^ 137) * 0.000005;
        self.radius * (continent * 0.0013 + mountains * 0.0011 + regional)
    }

    pub fn surface_radius(&self, direction: DVec3) -> f64 {
        self.radius + self.elevation(direction.normalize_or_zero())
    }

    pub fn altitude(&self, position: DVec3) -> f64 {
        position.length() - self.surface_radius(position)
    }

    pub fn position(&self, direction: DVec3) -> DVec3 {
        direction * self.surface_radius(direction)
    }

    pub fn surface_normal(&self, d: DVec3) -> DVec3 {
        // A fixed physical derivative scale makes shading independent of patch LOD.
        let axis = if d.y.abs() < 0.9 { DVec3::Y } else { DVec3::X };
        let tangent = axis.cross(d).normalize();
        let bitangent = d.cross(tangent);
        let epsilon = 1e-6;
        let dx = self.position((d + tangent * epsilon).normalize())
            - self.position((d - tangent * epsilon).normalize());
        let dy = self.position((d + bitangent * epsilon).normalize())
            - self.position((d - bitangent * epsilon).normalize());
        dx.cross(dy).normalize()
    }

    pub fn surface_material(&self, d: DVec3, normal: DVec3) -> MaterialId {
        let h = self.elevation(d) / self.radius;
        if d.y.abs() + h.max(0.0) * 220.0 > 0.87 {
            MaterialId::Ice
        } else if h < -0.00018 {
            MaterialId::Basalt
        } else if h < -0.00008 {
            MaterialId::Sand
        } else if h > 0.00055 || normal.dot(d) < 0.96 {
            MaterialId::Granite
        } else {
            MaterialId::Soil
        }
    }

    pub fn appearance(&self, d: DVec3) -> ([f32; 3], [f32; 3]) {
        let normal = self.surface_normal(d);
        (
            normal.as_vec3().to_array(),
            self.surface_material(d, normal).color(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deterministic_order_independent_and_seeded() {
        let a = Planet {
            radius: 6_371_000.0,
            seed: 928371,
        };
        let b = Planet { seed: 77, ..a };
        let directions: Vec<_> = (0..1000)
            .map(|i| DVec3::new(i as f64 * 0.13 - 50.0, 13.0, 7.0).normalize())
            .collect();
        let samples: Vec<_> = directions.iter().map(|&d| a.surface_radius(d)).collect();
        for (i, &d) in directions.iter().enumerate().rev() {
            assert_eq!(samples[i], a.surface_radius(d));
            assert!((samples[i] - a.radius).abs() < a.radius * 0.003);
        }
        assert_ne!(a.surface_radius(DVec3::X), b.surface_radius(DVec3::X));
    }
}
