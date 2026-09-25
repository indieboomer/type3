use glam::{DQuat, DVec3, Mat4, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraMode {
    Orbit,
    FreeFlight,
}

pub struct Camera {
    pub position: DVec3,
    pub orientation: DQuat,
    pub mode: CameraMode,
    pub radius: f64,
    pub surface_radius: f64,
    pub speed_scale: f64,
}

impl Camera {
    pub fn new(radius: f64) -> Self {
        Self {
            position: DVec3::new(0.0, 0.0, radius * 3.0),
            orientation: DQuat::IDENTITY,
            mode: CameraMode::Orbit,
            radius,
            surface_radius: radius,
            speed_scale: 1.0,
        }
    }

    pub fn altitude(&self) -> f64 {
        self.position.length() - self.radius
    }
    pub fn speed(&self) -> f64 {
        ((self.position.length() - self.surface_radius).abs() * 0.5).clamp(0.1, self.radius * 2.0)
            * self.speed_scale
    }
    pub fn forward(&self) -> DVec3 {
        self.orientation * -DVec3::Z
    }

    pub fn toggle_mode(&mut self) {
        self.mode = if self.mode == CameraMode::Orbit {
            CameraMode::FreeFlight
        } else {
            self.look_at_center();
            CameraMode::Orbit
        };
    }

    pub fn look_at_center(&mut self) {
        let z = self.position.normalize_or_zero();
        let up = if z.y.abs() > 0.99 { DVec3::Z } else { DVec3::Y };
        let x = up.cross(z).normalize_or_zero();
        let y = z.cross(x);
        self.orientation = DQuat::from_mat3(&glam::DMat3::from_cols(x, y, z));
    }

    pub fn rotate(&mut self, dx: f64, dy: f64) {
        let rotation = if self.mode == CameraMode::Orbit {
            DQuat::from_rotation_y(-dx * 0.004)
                * self.orientation
                * DQuat::from_rotation_x(-dy * 0.004)
        } else {
            self.orientation
                * DQuat::from_rotation_y(-dx * 0.0025)
                * DQuat::from_rotation_x(-dy * 0.0025)
        };
        self.orientation = rotation.normalize();
        if self.mode == CameraMode::Orbit {
            self.position = self.orientation * DVec3::Z * self.position.length();
        }
    }

    pub fn zoom(&mut self, lines: f64) {
        if self.mode == CameraMode::Orbit {
            let height = ((self.position.length() - self.surface_radius).max(0.25)
                * (-lines * 0.15).exp())
            .clamp(0.25, self.radius * 100.0);
            self.position = self.position.normalize() * (self.surface_radius + height);
        } else {
            self.speed_scale = (self.speed_scale * (lines * 0.2).exp()).clamp(0.01, 100.0);
        }
    }

    pub fn translate(&mut self, local: DVec3, seconds: f64, multiplier: f64) {
        if self.mode == CameraMode::FreeFlight {
            self.position +=
                self.orientation * local.normalize_or_zero() * self.speed() * multiplier * seconds;
        }
    }

    /// Positive input banks left (Q); negative banks right (E). Roll rotates
    /// the local frame around forward without changing position or heading.
    pub fn roll(&mut self, input: f64, seconds: f64) {
        if self.mode == CameraMode::FreeFlight {
            self.orientation =
                (self.orientation * DQuat::from_rotation_z(input * seconds * 1.2)).normalize();
        }
    }

    /// Translation is performed in f64 before vertices reach the GPU.
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        let projection = Mat4::perspective_infinite_reverse_rh(60_f32.to_radians(), aspect, 0.05);
        projection * Mat4::from_quat(self.orientation.conjugate().as_quat())
    }

    pub fn relative(&self, absolute: DVec3) -> Vec3 {
        (absolute - self.position).as_vec3()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::EARTH_RADIUS_METERS;

    #[test]
    fn roll_banks_local_frame_without_translation_or_heading_change() {
        let mut camera = Camera::new(EARTH_RADIUS_METERS);
        camera.toggle_mode();
        camera.rotate(123.0, -45.0);
        let position = camera.position;
        let forward = camera.forward();
        let orientation = camera.orientation;
        camera.roll(1.0, 0.5);
        assert_eq!(camera.position, position);
        assert!(camera.forward().distance(forward) < 1e-12);
        assert!((camera.orientation * DVec3::Y).distance(orientation * DVec3::Y) > 0.1);
        camera.roll(-1.0, 0.5);
        assert!(camera.orientation.dot(orientation).abs() > 1.0 - 1e-12);
    }

    #[test]
    fn camera_relative_precision_at_opposite_sides() {
        for sign in [-1.0, 1.0] {
            let mut camera = Camera::new(EARTH_RADIUS_METERS);
            camera.position = DVec3::X * sign * (EARTH_RADIUS_METERS + 0.5);
            assert!((camera.relative(camera.position + DVec3::X * 0.1).x - 0.1).abs() < 1e-7);
        }
    }

    #[test]
    fn reversed_depth_maps_near_to_one_and_far_toward_zero() {
        let camera = Camera::new(EARTH_RADIUS_METERS);
        let matrix = camera.view_projection(1.0);
        let near = matrix.project_point3(Vec3::new(0.0, 0.0, -0.05)).z;
        let far = matrix.project_point3(Vec3::new(0.0, 0.0, -1e8)).z;
        assert!((near - 1.0).abs() < 1e-6);
        assert!(far > 0.0 && far < 1e-8);
    }

    #[test]
    fn orbit_zoom_and_mode_changes_preserve_finite_state() {
        let mut camera = Camera::new(EARTH_RADIUS_METERS);
        for _ in 0..200 {
            camera.rotate(13.0, 7.0);
            camera.zoom(1.0);
        }
        assert!(camera.position.is_finite());
        assert!(camera.altitude() >= 0.249);
        let before = camera.position;
        camera.toggle_mode();
        assert_eq!(before, camera.position);
        camera.translate(DVec3::NEG_Z, 0.01, 1.0);
        assert!((before - camera.position).length() > 0.0);
        camera.toggle_mode();
        assert!(camera.forward().dot(-camera.position.normalize()) > 0.999999);
    }

    #[test]
    fn flight_uses_local_axes_and_surface_speed_without_diagonal_boost() {
        let mut camera = Camera::new(EARTH_RADIUS_METERS);
        camera.mode = CameraMode::FreeFlight;
        camera.position = DVec3::Z * (EARTH_RADIUS_METERS + 3001.0);
        camera.surface_radius = EARTH_RADIUS_METERS + 3000.0;
        assert_eq!(camera.speed(), 0.5);
        camera.orientation = DQuat::from_rotation_z(1.2);
        let before = camera.orientation;
        camera.rotate(100.0, 0.0);
        let expected = before * DQuat::from_rotation_y(-0.25);
        assert!(camera.forward().distance(expected * -DVec3::Z) < 1e-12);
        let start = camera.position;
        camera.translate(DVec3::new(1.0, 0.0, -1.0), 0.1, 1.0);
        assert!((camera.position.distance(start) - 0.05).abs() < 1e-8);
        let start = camera.position;
        camera.zoom(5.0);
        assert_eq!(camera.position, start);
        assert!(camera.speed_scale > 1.0);
    }
}
