//! Camera-independent canonical matter queries. No voxel storage or mutable RNG.
use super::{GENERATION_VERSION, Planet, hash, noise};
use crate::world::{
    CELL_SIZE_METERS, CellCoord, CellOverrideProvider, CellSample, EffectiveCellSample,
    GenerationVersion, MaterialId, SampleSource,
};
use glam::DVec3;

/// Radius is part of the world configuration, alongside seed and version.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanetIdentity {
    pub version: GenerationVersion,
    pub seed: u64,
    pub radius_m: f64,
}

pub trait PlanetField {
    /// Requires a finite planet-local position. Positive values are solid.
    fn sample_density(&self, position_m: DVec3) -> f64;
    fn sample_material(&self, position_m: DVec3) -> MaterialId;
    fn sample_cell(&self, cell: CellCoord) -> CellSample;
}

pub fn sample_effective_cell(
    field: &impl PlanetField,
    overrides: &impl CellOverrideProvider,
    cell: CellCoord,
) -> EffectiveCellSample {
    if let Some(sample) = overrides.get_override(cell) {
        EffectiveCellSample {
            sample,
            source: SampleSource::Override,
        }
    } else {
        EffectiveCellSample {
            sample: field.sample_cell(cell),
            source: SampleSource::Procedural,
        }
    }
}

impl Planet {
    /// Outer density crossing along a radial direction, for navigation only.
    /// Cavity geometry is still extracted from the full 3D field, not this root.
    pub fn outer_surface_radius(&self, direction: DVec3) -> f64 {
        let d = direction.try_normalize().unwrap_or(DVec3::Y);
        let macro_radius = self.surface_radius(d);
        let band = self.detail_base_scale() * 0.125;
        let mut hi = macro_radius + band;
        for i in 1..=64 {
            let mut lo = macro_radius + band - (2.0 * band) * (i as f64 / 64.0);
            if self.sample_density(d * lo) > 0.0 {
                for _ in 0..28 {
                    let mid = (lo + hi) * 0.5;
                    if self.sample_density(d * mid) > 0.0 {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                return (lo + hi) * 0.5;
            }
            hi = lo;
        }
        macro_radius
    }
    pub fn identity(&self) -> PlanetIdentity {
        PlanetIdentity {
            version: GENERATION_VERSION,
            seed: self.seed,
            radius_m: self.radius,
        }
    }

    pub fn detail_base_scale(&self) -> f64 {
        (self.radius * 0.01).clamp(0.2, 512.0)
    }

    /// Fixed finite fBm spectrum in physical metres. Fine layers never depend
    /// on the camera or mesh LOD. Stop at a >= 2-cell noise lattice scale.
    pub fn volumetric_detail(&self, position: DVec3) -> f64 {
        let mut wavelength = self.detail_base_scale();
        let mut amplitude = wavelength * 0.025;
        let mut value = 0.0;
        let seed = self.seed ^ hash(GENERATION_VERSION.0 as u64) ^ 0x726f636b;
        for octave in 0..16 {
            if wavelength < 2.0 * CELL_SIZE_METERS {
                break;
            }
            value += noise(position / wavelength, seed ^ hash(octave)) * amplitude;
            wavelength *= 0.5;
            amplitude *= 0.55;
        }
        value
    }

    fn density_and_depth(&self, position: DVec3) -> (f64, f64) {
        assert!(
            position.is_finite(),
            "matter query requires a finite position"
        );
        let distance = position.length();
        let direction = position.try_normalize().unwrap_or(DVec3::Y);
        // Fade directional elevation only in the deep interior so the centre
        // has a unique continuous value, independent of approach direction.
        let t = (distance / (self.radius * 0.5)).min(1.0);
        let depth = self.radius + self.elevation(direction) * t * t * (3.0 - 2.0 * t) - distance;
        // Detail has a bounded envelope; skip all high frequencies elsewhere.
        let band = self.detail_base_scale() * 0.125;
        let envelope = (1.0 - depth.abs() / band).max(0.0);
        let mut density = depth;
        if envelope > 0.0 {
            density += self.volumetric_detail(position) * envelope * envelope;
        }

        // Intersections of two 3D noise bands create underground tubes. Their
        // roof/floor are clamped to a shallow shell, keeping deep rock solid.
        let scale = (self.radius / 1000.0).min(1.0);
        let roof = 20.0 * scale;
        let floor = 80.0 * scale;
        let vertical_bound = (roof - depth).max(depth - floor);
        let cave = if vertical_bound < 24.0 * scale {
            let seed = self.seed ^ hash(GENERATION_VERSION.0 as u64) ^ 0x63617665;
            let p = position / (24.0 * scale);
            let tube = noise(p, seed)
                .abs()
                .max(noise(p + DVec3::splat(19.7), seed ^ 91).abs());
            ((tube - 0.13) * 24.0 * scale).max(vertical_bound)
        } else {
            // This bound dominates every possible tube value. Keeping it in
            // the CSG expression avoids a discontinuity at the shell boundary.
            vertical_bound
        };
        density = density.min(cave);
        (density, depth)
    }

    fn material_at(&self, position: DVec3, density: f64, macro_depth: f64) -> MaterialId {
        if density <= 0.0 {
            let atmosphere = (self.radius * 0.02).min(100_000.0);
            return if macro_depth >= -atmosphere {
                MaterialId::Air
            } else {
                MaterialId::Vacuum
            };
        }
        let depth_scale = (self.radius / 1000.0).min(1.0);
        if macro_depth > (self.radius * 0.2).min(5000.0) {
            return MaterialId::Basalt;
        }
        if macro_depth <= 2.0 * depth_scale {
            let d = position.try_normalize().unwrap_or(DVec3::Y);
            return self.surface_material(d, self.surface_normal(d));
        }
        let seed = self.seed ^ hash(GENERATION_VERSION.0 as u64) ^ 0x67656f6c;
        if noise(position / (150.0 * depth_scale), seed) > 0.2 {
            MaterialId::Limestone
        } else {
            MaterialId::Granite
        }
    }
}

impl PlanetField for Planet {
    fn sample_density(&self, position_m: DVec3) -> f64 {
        self.density_and_depth(position_m).0
    }

    fn sample_material(&self, position_m: DVec3) -> MaterialId {
        let (density, depth) = self.density_and_depth(position_m);
        self.material_at(position_m, density, depth)
    }

    fn sample_cell(&self, cell: CellCoord) -> CellSample {
        let position = cell.center_meters();
        let (density, depth) = self.density_and_depth(position);
        CellSample {
            density,
            material: self.material_at(position, density, depth),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{EARTH_RADIUS_METERS, EmptyOverrideProvider};
    fn planet() -> Planet {
        Planet {
            radius: EARTH_RADIUS_METERS,
            seed: 928371,
        }
    }

    #[test]
    fn arbitrary_depths_centre_atmosphere_and_space() {
        let p = planet();
        for direction in [
            DVec3::X,
            -DVec3::X,
            DVec3::Y,
            DVec3::new(1.0, 2.0, -3.0).normalize(),
        ] {
            let r = p.surface_radius(direction);
            for depth in [100.0, 10_000.0] {
                let position = direction * (r - depth);
                assert!(p.sample_density(position) > 0.0);
                assert!(!matches!(
                    p.sample_material(position),
                    MaterialId::Air | MaterialId::Vacuum
                ));
            }
            assert!(p.sample_density(direction * (r + 100.0)) < 0.0);
            assert_eq!(p.sample_material(direction * (r + 100.0)), MaterialId::Air);
            assert_eq!(
                p.sample_material(direction * (r + 200_000.0)),
                MaterialId::Vacuum
            );
        }
        assert!(p.sample_density(DVec3::ZERO).is_finite());
        assert!(p.sample_density(DVec3::ZERO) > 0.0);
        assert_eq!(p.sample_material(DVec3::ZERO), MaterialId::Basalt);
        for direction in [DVec3::X, -DVec3::X, DVec3::Y, DVec3::Z] {
            assert!(
                (p.sample_density(direction * 1e-6) - p.sample_density(DVec3::ZERO)).abs() < 1e-5
            );
        }
    }

    #[test]
    fn thousands_of_cells_repeat_independent_of_order() {
        let p = planet();
        let cells: Vec<_> = (0..4096)
            .map(|i| {
                let h = hash(i + 1);
                let d = DVec3::new(
                    (h as i32) as f64,
                    ((h >> 16) as i32) as f64,
                    ((h >> 32) as i32) as f64,
                )
                .normalize();
                CellCoord::from_meters(d * (p.surface_radius(d) + (i % 400) as f64 - 200.0))
                    .unwrap()
            })
            .collect();
        let samples: Vec<_> = cells.iter().map(|&c| p.sample_cell(c)).collect();
        for i in (0..cells.len()).rev() {
            let sample = p.sample_cell(cells[i]);
            assert_eq!(sample, samples[i]);
            assert_eq!(sample.density, p.sample_density(cells[i].center_meters()));
            assert_eq!(sample.material, p.sample_material(cells[i].center_meters()));
            assert_eq!(
                sample.density <= 0.0,
                matches!(sample.material, MaterialId::Air | MaterialId::Vacuum)
            );
        }
        assert_ne!(
            p.volumetric_detail(DVec3::new(1.2, 3.4, 5.6)),
            Planet { seed: 42, ..p }.volumetric_detail(DVec3::new(1.2, 3.4, 5.6))
        );
    }

    #[test]
    fn real_subsurface_cavities_are_not_a_heightmap() {
        let p = planet();
        let mut found = false;
        for i in 0..1024 {
            let d = DVec3::new(1.0, i as f64 * 0.00001, 0.12).normalize();
            let r = p.surface_radius(d);
            if p.sample_density(d * (r - 45.0)) < 0.0 {
                assert!(p.sample_density(d * (r - 15.0)) > 0.0);
                assert!(p.sample_density(d * (r - 100.0)) > 0.0);
                assert_eq!(p.sample_material(d * (r - 45.0)), MaterialId::Air);
                found = true;
                break;
            }
        }
        assert!(
            found,
            "expected an enclosed empty interval between solid roof and floor"
        );
    }

    #[test]
    fn overrides_win_and_empty_provider_preserves_procedural_samples() {
        struct Override {
            cell: CellCoord,
            sample: CellSample,
        }
        impl CellOverrideProvider for Override {
            fn get_override(&self, cell: CellCoord) -> Option<CellSample> {
                (cell == self.cell).then_some(self.sample)
            }
        }
        let p = planet();
        let cell = CellCoord { x: 0, y: 0, z: 0 };
        let empty = sample_effective_cell(&p, &EmptyOverrideProvider, cell);
        assert_eq!(empty.sample, p.sample_cell(cell));
        assert_eq!(empty.source, SampleSource::Procedural);
        let provider = Override {
            cell,
            sample: CellSample {
                density: -1.0,
                material: MaterialId::Air,
            },
        };
        let sample = sample_effective_cell(&p, &provider, cell);
        assert_eq!(sample.sample, provider.sample);
        assert_eq!(sample.source, SampleSource::Override);
        assert_eq!(
            sample_effective_cell(&p, &provider, CellCoord { x: 1, ..cell }).source,
            SampleSource::Procedural
        );
    }

    #[test]
    fn detail_is_bounded_has_fine_variation_and_surface_crossings() {
        let p = planet();
        for sign in [-1.0, 1.0] {
            let d = DVec3::X * sign;
            let r = p.surface_radius(d);
            let mut lo = r - 64.0;
            let mut hi = r + 64.0;
            for _ in 0..50 {
                let mid = (lo + hi) * 0.5;
                if p.sample_density(d * mid) > 0.0 {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            assert!((hi - lo) < 1e-7);
            assert!(p.sample_density(d * (hi + 0.1)) < 0.0);
            assert!(p.sample_density(d * (lo - 0.1)) > 0.0);
            let a = CellCoord::from_meters(d * r).unwrap();
            let b = CellCoord { x: a.x + 1, ..a };
            assert_ne!(p.sample_cell(a).density, p.sample_cell(b).density);
            assert!(
                p.volumetric_detail(d * r).abs() < p.detail_base_scale() * 0.025 / (1.0 - 0.55)
            );
        }
    }

    #[test]
    fn finite_queries_across_presets_and_extreme_cell_addresses() {
        for radius in [1.0, 1000.0, 50_000.0, 1_000_000.0, EARTH_RADIUS_METERS, 1e9] {
            let p = Planet { radius, ..planet() };
            assert_eq!(p.sample_material(DVec3::ZERO), MaterialId::Basalt);
            for cell in [
                CellCoord {
                    x: i64::MIN,
                    y: 0,
                    z: i64::MAX,
                },
                CellCoord::from_meters(p.position(DVec3::X)).unwrap(),
            ] {
                assert!(p.sample_cell(cell).density.is_finite());
            }
        }
    }

    #[test]
    fn shell_boundaries_are_continuous() {
        let p = planet();
        let direction = DVec3::new(1.0, 0.001, 0.12).normalize();
        let r = p.surface_radius(direction);
        for depth in [-64.0, -4.0, 20.0, 64.0, 80.0, 104.0] {
            let a = p.sample_density(direction * (r - depth - 1e-6));
            let b = p.sample_density(direction * (r - depth + 1e-6));
            assert!(
                (a - b).abs() < 1e-4,
                "discontinuous density at depth {depth}: {a} vs {b}"
            );
        }
    }

    #[test]
    fn sub_metre_detail_does_not_collapse_to_a_flat_interpolation() {
        let p = planet();
        let origin = p.position(DVec3::new(1.0, 0.1, 0.2).normalize());
        // A metre-long line has curvature from fine noise, even when macro
        // direction barely changes at this planetary radius.
        let a = p.volumetric_detail(origin);
        let b = p.volumetric_detail(origin + DVec3::Y);
        let mut deviation = 0.0_f64;
        for i in 1..10 {
            let t = i as f64 / 10.0;
            deviation = deviation
                .max((p.volumetric_detail(origin + DVec3::Y * t) - (a + (b - a) * t)).abs());
        }
        assert!(
            deviation > 0.001,
            "fine detail must contribute millimetre-scale curvature within a metre"
        );
    }
}
