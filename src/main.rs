use anyhow::{Context, Result, bail};
use glam::DVec3;
use std::{collections::HashSet, time::Instant};
use type3::{
    app::camera::{Camera, CameraMode},
    renderer::{
        sphere::{Sphere, Vertex},
        vulkan::VulkanRenderer,
    },
    world::{CellCoord, EARTH_RADIUS_METERS},
};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};

struct Config {
    radius: f64,
    validation: bool,
    smoke_frames: Option<u64>,
}

impl Config {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>> {
        let mut config = Self {
            radius: EARTH_RADIUS_METERS,
            validation: false,
            smoke_frames: None,
        };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--radius" => {
                    config.radius = args
                        .next()
                        .context("--radius requires metres")?
                        .parse()
                        .context("Invalid radius")?
                }
                "--preset" => {
                    config.radius = match args.next().as_deref() {
                        Some("tiny") => 1_000.0,
                        Some("small") => 50_000.0,
                        Some("large") => 1_000_000.0,
                        Some("earth") => EARTH_RADIUS_METERS,
                        _ => bail!("--preset must be tiny, small, large or earth"),
                    }
                }
                "--validation" => config.validation = true,
                "--smoke-test" => {
                    let frames = args
                        .next()
                        .context("--smoke-test requires a frame count (at least 120)")?
                        .parse::<u64>()?;
                    anyhow::ensure!(frames >= 120, "Smoke test requires at least 120 frames");
                    config.smoke_frames = Some(frames);
                }
                "--help" | "-h" => {
                    println!(
                        "Type3 — Vulkan sphere foundation\n\ncargo run --release -- [OPTIONS]\n\n  --preset tiny|small|large|earth   Planet radius (default earth)\n  --radius METRES                  Custom radius, 1..1e9 m\n  --validation                     Require Khronos validation layer\n  --smoke-test FRAMES              Script navigation/resize, then exit (>=120)\n\nLMB drag: orbit | wheel: zoom | Tab: free flight\nRMB hold: mouse look in flight | WASD: move | Q/E: down/up\nShift: 10x speed | Ctrl: 0.1x speed | Home: reset | Esc: exit"
                    );
                    return Ok(None);
                }
                _ => bail!("Unknown argument {arg}; use --help"),
            }
        }
        anyhow::ensure!(
            config.radius.is_finite() && (1.0..=1e9).contains(&config.radius),
            "Radius must be finite and in 1..1e9 metres"
        );
        Ok(Some(config))
    }
}

struct Running {
    // Renderer must drop before its native window.
    renderer: VulkanRenderer,
    window: Window,
    egui: egui::Context,
    egui_input: egui_winit::State,
    sphere: Sphere,
    vertices: Vec<Vertex>,
    camera: Camera,
    keys: HashSet<KeyCode>,
    dragging: bool,
    looking: bool,
    last_cursor: Option<(f64, f64)>,
    last_frame: Instant,
    frame_ms: f64,
    frames: u64,
}

impl Running {
    fn new(event_loop: &ActiveEventLoop, config: &Config) -> Result<Self> {
        let window = event_loop.create_window(
            Window::default_attributes()
                .with_title("Type3 · One-litre planet · Vulkan foundation")
                .with_inner_size(LogicalSize::new(1280, 800)),
        )?;
        let sphere = Sphere::new(config.radius, 64);
        let renderer = VulkanRenderer::new(
            &window,
            sphere.positions.len(),
            &sphere.indices,
            config.validation,
        )?;
        eprintln!(
            "Vulkan device: {} | radius: {} m | {} triangles",
            renderer.device_name,
            config.radius,
            sphere.indices.len() / 3
        );
        let egui = egui::Context::default();
        let egui_input = egui_winit::State::new(
            egui.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            None,
        );
        Ok(Self {
            renderer,
            window,
            egui,
            egui_input,
            sphere,
            vertices: Vec::new(),
            camera: Camera::new(config.radius),
            keys: HashSet::new(),
            dragging: false,
            looking: false,
            last_cursor: None,
            last_frame: Instant::now(),
            frame_ms: 16.7,
            frames: 0,
        })
    }

    fn release_input(&mut self) {
        self.keys.clear();
        self.dragging = false;
        self.looking = false;
        self.last_cursor = None;
        let _ = self.window.set_cursor_grab(CursorGrabMode::None);
        self.window.set_cursor_visible(true);
    }

    fn frame(&mut self, config: &Config) -> Result<()> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_frame).as_secs_f64();
        self.last_frame = now;
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        if self.renderer.needs_resize {
            self.renderer.resize(&self.window)?;
        }
        self.frame_ms = self.frame_ms * 0.95 + elapsed * 1000.0 * 0.05;
        let pressed = |key| if self.keys.contains(&key) { 1.0 } else { 0.0 };
        let movement = DVec3::new(
            pressed(KeyCode::KeyD) - pressed(KeyCode::KeyA),
            pressed(KeyCode::KeyE) - pressed(KeyCode::KeyQ),
            pressed(KeyCode::KeyS) - pressed(KeyCode::KeyW),
        );
        let multiplier = if self.keys.contains(&KeyCode::ShiftLeft)
            || self.keys.contains(&KeyCode::ShiftRight)
        {
            10.0
        } else if self.keys.contains(&KeyCode::ControlLeft)
            || self.keys.contains(&KeyCode::ControlRight)
        {
            0.1
        } else {
            1.0
        };
        if !self.egui.wants_keyboard_input() {
            self.camera
                .translate(movement, elapsed.min(0.1), multiplier);
        }
        if config.smoke_frames.is_some() {
            match self.frames {
                20 => {
                    let _ = self.window.request_inner_size(LogicalSize::new(1000, 700));
                }
                40 => self.camera.toggle_mode(),
                60 => {
                    self.camera.position = DVec3::new(-config.radius - 0.5, 0.0, 0.0);
                    self.camera.toggle_mode();
                }
                80 => {
                    self.camera = Camera::new(config.radius);
                    let _ = self.window.request_inner_size(LogicalSize::new(1280, 800));
                }
                _ => (),
            }
            if self.frames < 40 {
                self.camera.rotate(1.0, 0.1);
            }
        }
        let raw_input = self.egui_input.take_egui_input(&self.window);
        let output = self.egui.run(raw_input, |ctx| {
            egui::Window::new("TYPE3 / Vulkan foundation").default_pos([16.0, 16.0]).default_width(335.0).show(ctx, |ui| {
                ui.label("Milestone 0 · fixed-resolution test sphere");
                ui.small("Procedural terrain and volumetric matter are not implemented yet.");
                ui.separator();
                ui.label(format!("{:.0} FPS  /  {:.2} ms (CPU frame interval)", 1000.0 / self.frame_ms.max(0.001), self.frame_ms));
                ui.label(format!("GPU: {}", self.renderer.device_name));
                ui.label("Vulkan · reversed-Z · f64 CPU / relative f32 GPU");
                ui.separator();
                ui.label(format!("Camera: {:?}", self.camera.mode));
                ui.monospace(format!("X {:>16.3} m\nY {:>16.3} m\nZ {:>16.3} m", self.camera.position.x, self.camera.position.y, self.camera.position.z));
                ui.label(format!("Radius: {:.3} km", self.camera.radius / 1000.0));
                ui.label(format!("Centre distance: {:.3} m", self.camera.position.length()));
                ui.label(format!("Reference altitude: {:.3} m", self.camera.altitude()));
                ui.label(format!("Sphere altitude: {:.3} m", self.camera.altitude()));
                ui.label(format!("Flight speed: {:.3} m/s", self.camera.speed() * multiplier));
                if let Some(cell) = CellCoord::from_meters(self.camera.position) {
                    ui.separator();
                    ui.label("Camera's spatial cell (0.10 m lattice)");
                    ui.monospace(format!("({}, {}, {})", cell.x, cell.y, cell.z));
                    ui.small("Address only; not a terrain pick or matter sample.");
                }
                ui.separator();
                ui.label(format!("{} vertices / {} triangles", self.sphere.positions.len(), self.sphere.indices.len() / 3));
                ui.small("GPU timing, terrain LOD and cell inspection: pending");
                if ui.button("Toggle orbit / flight [Tab]").clicked() { self.camera.toggle_mode(); }
                if ui.button("Reset to orbit [Home]").clicked() { self.camera = Camera::new(config.radius); }
                ui.separator();
                ui.small("Orbit: LMB drag · wheel zoom\nFlight: hold RMB to look · WASD move · Q/E down/up\nShift faster · Ctrl slower · Esc exit");
            });
        });
        self.egui_input
            .handle_platform_output(&self.window, output.platform_output);
        let primitives = self.egui.tessellate(output.shapes, output.pixels_per_point);
        self.sphere
            .write_relative_vertices(&self.camera, &mut self.vertices);
        let aspect = self.renderer.extent.width as f32 / self.renderer.extent.height as f32;
        if self.renderer.draw(
            &self.vertices,
            self.camera.view_projection(aspect).to_cols_array(),
            &primitives,
            &output.textures_delta,
            output.pixels_per_point,
        )? {
            self.frames += 1;
        }
        Ok(())
    }
}

struct App {
    config: Config,
    running: Option<Running>,
    error: Option<anyhow::Error>,
}

impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_none() {
            match Running::new(event_loop, &self.config) {
                Ok(running) => self.running = Some(running),
                Err(error) => self.fail(event_loop, error),
            }
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.running.take();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(running) = self.running.as_mut() else {
            return;
        };
        let response = running.egui_input.on_window_event(&running.window, &event);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                running.renderer.needs_resize = true
            }
            WindowEvent::Focused(false) => running.release_input(),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    if event.state == ElementState::Released {
                        running.keys.remove(&key);
                    } else if !response.consumed {
                        running.keys.insert(key);
                        if !event.repeat {
                            match key {
                                KeyCode::Escape => event_loop.exit(),
                                KeyCode::Tab => {
                                    running.release_input();
                                    running.camera.toggle_mode();
                                }
                                KeyCode::Home => {
                                    running.release_input();
                                    running.camera = Camera::new(self.config.radius);
                                }
                                _ => (),
                            }
                        }
                    }
                }
            }
            WindowEvent::MouseInput { button, state, .. } => {
                let pressed = state == ElementState::Pressed;
                if button == MouseButton::Left && (!pressed || !response.consumed) {
                    running.dragging = pressed;
                }
                if button == MouseButton::Right && (!pressed || !response.consumed) {
                    running.looking = pressed && running.camera.mode == CameraMode::FreeFlight;
                    if running.looking {
                        let _ = running
                            .window
                            .set_cursor_grab(CursorGrabMode::Locked)
                            .or_else(|_| running.window.set_cursor_grab(CursorGrabMode::Confined));
                        running.window.set_cursor_visible(false);
                    } else {
                        let _ = running.window.set_cursor_grab(CursorGrabMode::None);
                        running.window.set_cursor_visible(true);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some((x, y)) = running.last_cursor
                    && running.dragging
                    && running.camera.mode == CameraMode::Orbit
                {
                    running.camera.rotate(position.x - x, position.y - y);
                }
                running.last_cursor = Some((position.x, position.y));
            }
            WindowEvent::MouseWheel { delta, .. } if !response.consumed => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                running.camera.zoom(lines);
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = running.frame(&self.config) {
                    self.fail(event_loop, error);
                } else if self
                    .config
                    .smoke_frames
                    .is_some_and(|limit| running.frames >= limit)
                {
                    eprintln!(
                        "Smoke test completed: {} presented frames, orbit/flight, opposite-side relocation, two resizes",
                        running.frames
                    );
                    event_loop.exit();
                }
            }
            _ => (),
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let (Some(running), DeviceEvent::MouseMotion { delta }) = (self.running.as_mut(), event)
            && running.looking
        {
            running.camera.rotate(delta.0, delta.1);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(running) = &self.running {
            let size = running.window.inner_size();
            if size.width > 0 && size.height > 0 {
                running.window.request_redraw();
            }
        }
    }
}

fn main() -> Result<()> {
    let Some(config) = Config::parse(std::env::args().skip(1))? else {
        return Ok(());
    };
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App {
        config,
        running: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    app.running.take();
    if let Some(error) = app.error {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Option<Config>> {
        Config::parse(args.iter().map(|s| s.to_string()))
    }
    #[test]
    fn command_line_validates_radius_and_presets() {
        assert_eq!(parse(&[]).unwrap().unwrap().radius, EARTH_RADIUS_METERS);
        assert_eq!(
            parse(&["--preset", "tiny"]).unwrap().unwrap().radius,
            1000.0
        );
        for args in [
            &["--radius", "NaN"][..],
            &["--radius", "-2"],
            &["--radius"],
            &["--preset", "unknown"],
            &["--smoke-test", "1"],
            &["--unknown"],
        ] {
            assert!(parse(args).is_err());
        }
    }
}
