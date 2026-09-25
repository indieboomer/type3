//! Canonical addressing, independent of rendering. No planetary cell storage.
use glam::DVec3;

pub const CELL_SIZE_METERS: f64 = 0.1;
pub const EARTH_RADIUS_METERS: f64 = 6_371_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellCoord {
    pub x: i64,
    pub y: i64,
    pub z: i64,
}

impl CellCoord {
    /// Returns None for non-finite or unaddressable positions, never saturates.
    pub fn from_meters(position: DVec3) -> Option<Self> {
        let p = (position / CELL_SIZE_METERS).floor();
        if !p.is_finite()
            || p.min_element() < i64::MIN as f64
            || p.max_element() >= -(i64::MIN as f64)
        {
            return None;
        }
        Some(Self {
            x: p.x as i64,
            y: p.y as i64,
            z: p.z as i64,
        })
    }

    pub fn center_meters(self) -> DVec3 {
        (DVec3::new(self.x as f64, self.y as f64, self.z as f64) + DVec3::splat(0.5))
            * CELL_SIZE_METERS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_addressing_including_negative_coordinates() {
        let cell = CellCoord::from_meters(DVec3::new(0.19, -0.01, -0.11)).unwrap();
        assert_eq!(cell, CellCoord { x: 1, y: -1, z: -2 });
        assert!((cell.center_meters() - DVec3::new(0.15, -0.05, -0.15)).length() < 1e-12);
        assert_eq!(CellCoord::from_meters(cell.center_meters()), Some(cell));
        assert!(CellCoord::from_meters(DVec3::splat(f64::NAN)).is_none());
        assert!(CellCoord::from_meters(DVec3::splat(1e30)).is_none());
    }

    #[test]
    fn adjacent_litres_remain_distinct_on_both_sides_of_earth() {
        for sign in [-1, 1] {
            let a = CellCoord {
                x: sign * 63_710_000,
                y: 0,
                z: 0,
            };
            let b = CellCoord { x: a.x + 1, ..a };
            assert_eq!(CellCoord::from_meters(a.center_meters()), Some(a));
            assert_eq!(CellCoord::from_meters(b.center_meters()), Some(b));
            let relative = (b.center_meters() - a.center_meters()).as_vec3();
            assert!((relative.x - 0.1).abs() < 1e-7);
        }
    }
}
