use anyhow::{Context, Result, bail};
use glam::DVec3;
use std::{collections::HashSet, time::Instant};
use type3::{
    app::camera::{Camera, CameraMode},
    planet::{GENERATION_VERSION, Planet, field::sample_effective_cell},
    renderer::{
        sphere::Vertex,
        terrain::{TerrainMesh, TerrainWorker},
        vulkan::VulkanRenderer,
    },
    voxel::{self, Clipmap, DrawBatch, SceneDraw, VolumeWorker},
    world::{CellCoord, EARTH_RADIUS_METERS, EffectiveCellSample, EmptyOverrideProvider},
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
    seed: u64,
    validation: bool,
    smoke_frames: Option<u64>,
    sample_cell: Option<CellCoord>,
}

impl Config {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>> {
        let mut config = Self {
            radius: EARTH_RADIUS_METERS,
            seed: 928371,
            validation: false,
            smoke_frames: None,
            sample_cell: None,
        };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--sample-cell" => {
                    let mut coordinate = || -> Result<i64> {
                        args.next()
                            .context("--sample-cell requires X Y Z integer coordinates")?
                            .parse()
                            .context("Invalid cell coordinate")
                    };
                    config.sample_cell = Some(CellCoord {
                        x: coordinate()?,
                        y: coordinate()?,
                        z: coordinate()?,
                    });
                }
                "--seed" => {
                    config.seed = args
                        .next()
                        .context("--seed requires an integer")?
                        .parse()
                        .context("Invalid seed")?
                }
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
                        "Type3 — Procedural macro planet\n\ncargo run --release -- [OPTIONS]\n\n  --preset tiny|small|large|earth   Planet radius (default earth)\n  --radius METRES                  Custom radius, 1..1e9 m\n  --seed INTEGER                   Deterministic planet seed (default 928371)\n  --sample-cell X Y Z              Query a 10 cm cell without opening Vulkan\n  --validation                     Require Khronos validation layer\n  --smoke-test FRAMES              Script navigation/resize, then exit (>=120)\n\nLMB drag: orbit | wheel: zoom | Tab: free flight\nMouse: look in flight | WASD: move | Q/E: roll left/right | Space/C: up/down | wheel: speed\nShift: 10x speed | Ctrl: 0.1x speed | Home: reset | Esc: release mouse, then exit"
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
    sphere: TerrainMesh,
    terrain: TerrainWorker,
    planet: Planet,
    freeze_lod: bool,
    vertices: Vec<Vertex>,
    camera: Camera,
    keys: HashSet<KeyCode>,
    dragging: bool,
    looking: bool,
    last_cursor: Option<(f64, f64)>,
    last_frame: Instant,
    frame_ms: f64,
    frames: u64,
    smoke_surface_verified: bool,
    smoke_surface_frame: u64,
    smoke_flight_start: DVec3,
    query_cell: CellCoord,
    query_result: Option<(CellCoord, EffectiveCellSample)>,
    query_repeat_matches: Option<bool>,
    volume_worker: VolumeWorker,
    volume: Option<Clipmap>,
    volume_blend: f32,
    volumes_enabled: bool,
    noclip: bool,
    indices: Vec<u32>,
    batches: Vec<DrawBatch>,
    smoke_roll_position: DVec3,
    smoke_roll_forward: DVec3,
    smoke_roll_up: DVec3,
}

// These are application controls even when egui has keyboard focus. In
// particular, egui-winit always consumes Tab for widget focus navigation.
fn camera_shortcut(key: KeyCode) -> bool {
    matches!(key, KeyCode::Tab | KeyCode::Home | KeyCode::Escape)
}

fn camera_key_allowed(key: KeyCode, consumed: bool) -> bool {
    camera_shortcut(key) || !consumed
}

impl Running {
    fn new(event_loop: &ActiveEventLoop, config: &Config) -> Result<Self> {
        let window = event_loop.create_window(
            Window::default_attributes()
                .with_title("Type3 · One-litre planet · Procedural planet")
                .with_inner_size(LogicalSize::new(1280, 800)),
        )?;
        let planet = Planet {
            radius: config.radius,
            seed: config.seed,
        };
        let (terrain, sphere) = TerrainWorker::new(
            planet,
            Camera::new(config.radius).position,
            window.inner_size().height as f64,
        );
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
            terrain,
            planet,
            freeze_lod: false,
            vertices: Vec::new(),
            camera: Camera::new(config.radius),
            keys: HashSet::new(),
            dragging: false,
            looking: false,
            last_cursor: None,
            last_frame: Instant::now(),
            frame_ms: 16.7,
            frames: 0,
            smoke_surface_verified: false,
            smoke_surface_frame: 0,
            smoke_flight_start: DVec3::ZERO,
            query_cell: CellCoord { x: 0, y: 0, z: 0 },
            query_result: None,
            query_repeat_matches: None,
            volume_worker: VolumeWorker::new(planet),
            volume: None,
            volume_blend: 0.0,
            volumes_enabled: true,
            noclip: false,
            indices: Vec::new(),
            batches: Vec::new(),
            smoke_roll_position: DVec3::ZERO,
            smoke_roll_forward: DVec3::ZERO,
            smoke_roll_up: DVec3::ZERO,
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

    fn capture_mouse(&mut self) {
        if self.camera.mode == CameraMode::FreeFlight {
            self.looking = self
                .window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok();
            self.window.set_cursor_visible(!self.looking);
            if self.looking {
                // A scene click may have entered egui before capture. Clear that
                // press so the hidden pointer cannot keep a widget dragging.
                let position = self.egui.pointer_hover_pos().unwrap_or_default();
                let input = self.egui_input.egui_input_mut();
                input.events.clear();
                for button in [egui::PointerButton::Primary, egui::PointerButton::Secondary] {
                    input.events.push(egui::Event::PointerButton {
                        pos: position,
                        button,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                input.events.push(egui::Event::PointerGone);
            }
        }
    }

    fn toggle_flight(&mut self) {
        self.release_input();
        self.camera.toggle_mode();
        self.capture_mouse();
    }

    /// Returns true only when the application should exit.
    fn handle_key(
        &mut self,
        key: KeyCode,
        state: ElementState,
        repeat: bool,
        consumed: bool,
    ) -> bool {
        if state == ElementState::Released {
            self.keys.remove(&key);
        } else if camera_key_allowed(key, consumed) {
            self.keys.insert(key);
            if !repeat {
                match key {
                    KeyCode::Escape => {
                        if self.looking {
                            self.release_input();
                        } else {
                            return true;
                        }
                    }
                    KeyCode::Tab => self.toggle_flight(),
                    KeyCode::Home => {
                        self.release_input();
                        self.camera = Camera::new(self.planet.radius);
                    }
                    _ => (),
                }
            }
        }
        false
    }

    fn handle_mouse_motion(&mut self, delta: (f64, f64)) {
        if self.looking {
            self.camera.rotate(delta.0, delta.1);
        }
    }

    fn keep_above_surface(&mut self) {
        let direction = self.camera.position.try_normalize().unwrap_or(DVec3::Z);
        self.camera.surface_radius = self.planet.surface_radius(direction);
        if self.planet.altitude(self.camera.position).abs() < voxel::ACTIVATION_ALTITUDE + 128.0 {
            self.camera.surface_radius = self.planet.outer_surface_radius(direction);
        }
        let floor = self.camera.surface_radius + 0.5;
        if !self.noclip && self.camera.position.length() < floor {
            self.camera.position = direction * floor;
        }
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
        self.keep_above_surface();
        let pressed = |key| if self.keys.contains(&key) { 1.0 } else { 0.0 };
        let movement = DVec3::new(
            pressed(KeyCode::KeyD) - pressed(KeyCode::KeyA),
            pressed(KeyCode::Space) - pressed(KeyCode::KeyC),
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
        if self.looking {
            self.camera.roll(
                pressed(KeyCode::KeyQ) - pressed(KeyCode::KeyE),
                elapsed.min(0.1),
            );
            self.camera
                .translate(movement, elapsed.min(0.1), multiplier);
        }
        if config.smoke_frames.is_some() {
            match self.frames {
                20 => {
                    let _ = self.window.request_inner_size(LogicalSize::new(1000, 700));
                }
                40 => {
                    // Reproduce egui consuming Tab, then use the real key and
                    // mouse handlers instead of moving the camera directly.
                    self.handle_key(KeyCode::Tab, ElementState::Pressed, false, true);
                    anyhow::ensure!(
                        self.camera.mode == CameraMode::FreeFlight && self.looking,
                        "Tab did not activate flight and capture the pointer"
                    );
                    self.smoke_flight_start = self.camera.position;
                    self.handle_key(KeyCode::KeyW, ElementState::Pressed, false, false);
                    let before = self.camera.forward();
                    self.handle_mouse_motion((24.0, -12.0));
                    anyhow::ensure!(
                        before.distance(self.camera.forward()) > 0.01,
                        "Mouse input did not rotate the flight camera"
                    );
                }
                45 => {
                    self.handle_key(KeyCode::KeyW, ElementState::Released, false, false);
                    self.smoke_roll_position = self.camera.position;
                    self.smoke_roll_forward = self.camera.forward();
                    self.smoke_roll_up = self.camera.orientation * DVec3::Y;
                    self.handle_key(KeyCode::KeyQ, ElementState::Pressed, false, false);
                }
                50 => {
                    anyhow::ensure!(
                        self.camera.position.distance(self.smoke_roll_position) < 1e-6,
                        "Q translated the camera instead of rolling"
                    );
                    anyhow::ensure!(
                        self.camera.forward().distance(self.smoke_roll_forward) < 1e-10,
                        "Q changed the heading instead of rolling"
                    );
                    anyhow::ensure!(
                        (self.camera.orientation * DVec3::Y).distance(self.smoke_roll_up) > 0.001,
                        "Q did not roll through keyboard input"
                    );
                    self.handle_key(KeyCode::KeyQ, ElementState::Released, false, false);
                    self.handle_key(KeyCode::KeyE, ElementState::Pressed, false, false);
                }
                60 => {
                    anyhow::ensure!(
                        self.camera.position.distance(self.smoke_flight_start) > 1.0,
                        "Held W did not move the camera through the frame input path"
                    );
                    self.handle_key(KeyCode::KeyW, ElementState::Released, false, true);
                    self.release_input();
                    self.camera.position =
                        -DVec3::X * (self.planet.outer_surface_radius(-DVec3::X) + 0.5);
                    self.camera.toggle_mode();
                }
                _ => (),
            }
            if self.frames < 40 {
                self.camera.rotate(1.0, 0.1);
            }
        }
        self.keep_above_surface();
        let volume_position = (self.volumes_enabled
            && self.planet.altitude(self.camera.position).abs() < voxel::ACTIVATION_ALTITUDE)
            .then_some(self.camera.position);
        if !self.freeze_lod || volume_position.is_none() {
            self.volume_worker.update(volume_position, &mut self.volume);
        }
        let desired_blend = if self.volume.is_some() {
            ((voxel::ACTIVATION_ALTITUDE - self.planet.altitude(self.camera.position).abs()) / 60.0)
                .clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let blend_step = (elapsed / 0.35).min(1.0) as f32;
        self.volume_blend += (desired_blend - self.volume_blend).clamp(-blend_step, blend_step);
        if !self.freeze_lod
            && let Some(mesh) =
                self.terrain
                    .update(self.planet, self.camera.position, size.height as f64)
        {
            self.sphere = mesh;
        }
        if config.smoke_frames.is_some() && self.frames >= 80 && !self.smoke_surface_verified {
            anyhow::ensure!(
                self.frames < 20_000,
                "Terrain worker did not deliver the near-surface smoke mesh"
            );
            if self.sphere.source_position.distance(self.camera.position) < 0.01
                && self
                    .volume
                    .as_ref()
                    .is_some_and(|v| v.source_position.distance(self.camera.position) < 0.01)
            {
                let volume = self.volume.as_ref().unwrap();
                anyhow::ensure!(
                    volume.levels[0]
                        .bricks
                        .iter()
                        .any(|b| !b.indices.is_empty()),
                    "No 0.10 m surface geometry generated"
                );
                eprintln!(
                    "Volume ready: {} levels, {} triangles, {} density samples, {:.1} ms, {:.2} MiB mesh, finest 0.10 m",
                    volume.levels.len(),
                    volume.triangles,
                    volume.samples,
                    volume.generation_ms,
                    volume.bytes as f64 / 1048576.0
                );
                eprintln!(
                    "Surface LOD ready: {} patches, depth {}, {:.1} ms generation, {:.3} m altitude",
                    self.sphere.patches,
                    self.sphere.depth,
                    self.sphere.generation_ms,
                    self.planet.altitude(self.camera.position)
                );
                self.smoke_surface_verified = true;
                self.smoke_surface_frame = self.frames;
            }
        }
        if config.smoke_frames.is_some()
            && self.smoke_surface_verified
            && self.camera.altitude() < config.radius
        {
            // The delivered surface mesh was presented on the previous frame.
            if self.frames > self.smoke_surface_frame + 180 && !self.terrain.busy {
                self.camera = Camera::new(config.radius);
                let _ = self.window.request_inner_size(LogicalSize::new(1280, 800));
            }
        }
        let raw_input = self.egui_input.take_egui_input(&self.window);
        let output = self.egui.clone().run(raw_input, |ctx| {
            egui::Window::new("TYPE3 / Procedural planet").default_pos([16.0, 16.0]).default_width(335.0).vscroll(true).show(ctx, |ui| {
                ui.label("Milestone 4 · near-field volumetric terrain");
                ui.small("Canonical 3D density · nested bricks · finest 0.10 m");
                ui.label(format!("Seed: {} / generation v{}", self.planet.seed, GENERATION_VERSION));
                ui.separator();
                ui.label(format!("{:.0} FPS  /  {:.2} ms (CPU frame interval)", 1000.0 / self.frame_ms.max(0.001), self.frame_ms));
                ui.label(format!("GPU: {}", self.renderer.device_name));
                ui.label("Vulkan · reversed-Z · f64 CPU / relative f32 GPU");
                ui.separator();
                ui.label(format!("Camera: {:?}", self.camera.mode));
                ui.label(if self.looking { "Flight controls active: WASD + mouse" } else if self.camera.mode == CameraMode::FreeFlight { "Flight paused: click the scene to resume" } else { "Press Tab to fly with WASD + mouse" });
                ui.monospace(format!("X {:>16.3} m\nY {:>16.3} m\nZ {:>16.3} m", self.camera.position.x, self.camera.position.y, self.camera.position.z));
                ui.label(format!("Radius: {:.3} km", self.camera.radius / 1000.0));
                ui.label(format!("Centre distance: {:.3} m", self.camera.position.length()));
                ui.label(format!("Reference altitude: {:.3} m", self.camera.altitude()));
                ui.label(format!("Surface altitude: {:.3} m", self.camera.position.length()-self.camera.surface_radius));
                ui.label(format!("Flight speed: {:.3} m/s", self.camera.speed() * multiplier));
                if let Some(cell) = CellCoord::from_meters(self.camera.position) {
                    ui.separator();
                    ui.label("Camera's spatial cell (0.10 m lattice)");
                    ui.monospace(format!("({}, {}, {})", cell.x, cell.y, cell.z));
                    let sample = sample_effective_cell(&self.planet, &EmptyOverrideProvider, cell);
                    ui.label(format!("{:?} / density {:+.6} / {:?}", sample.sample.material, sample.sample.density, sample.source));
                    ui.small("Cell-centre matter sample; not a terrain ray pick.");
                }
                ui.collapsing("Query any 10 cm cell", |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut self.query_cell.x).prefix("X "));
                        ui.add(egui::DragValue::new(&mut self.query_cell.y).prefix("Y "));
                        ui.add(egui::DragValue::new(&mut self.query_cell.z).prefix("Z "));
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Camera").clicked() && let Some(cell) = CellCoord::from_meters(self.camera.position) { self.query_cell = cell; }
                        if ui.button("Centre").clicked() { self.query_cell = CellCoord { x: 0, y: 0, z: 0 }; }
                        if ui.button("100 m deep").clicked() {
                            let d = self.camera.position.normalize();
                            let depth = 100.0_f64.min(self.planet.radius * 0.5);
                            self.query_cell = CellCoord::from_meters(d * (self.planet.surface_radius(d) - depth)).unwrap();
                        }
                    });
                    if ui.button("Sample / repeat query").clicked() {
                        let result = sample_effective_cell(&self.planet, &EmptyOverrideProvider, self.query_cell);
                        self.query_repeat_matches = self.query_result.filter(|(cell, _)| *cell == self.query_cell).map(|(_, previous)| previous == result);
                        self.query_result = Some((self.query_cell, result));
                    }
                    if let Some((cell, result)) = self.query_result {
                        ui.monospace(format!("Cell ({}, {}, {})\nCentre {:?} m", cell.x, cell.y, cell.z, cell.center_meters().to_array()));
                        ui.label(format!("{:?} / density {:+.9} / {:?}", result.sample.material, result.sample.density, result.source));
                    }
                    if let Some(matches) = self.query_repeat_matches { ui.label(if matches { "Repeated query: identical" } else { "Repeated query: CHANGED" }); }
                    ui.small("Signed density: positive solid, zero/negative empty. Not mass density.");
                });
                ui.separator();
                ui.label(format!("{} vertices / {} triangles", self.sphere.positions.len(), self.sphere.indices.len() / 3));
                ui.label(format!("{} patches / depth {} / generation {:.1} ms", self.sphere.patches, self.sphere.depth, self.sphere.generation_ms));
                ui.label(if self.terrain.busy { "Terrain worker: generating" } else { "Terrain worker: idle" });
                ui.checkbox(&mut self.freeze_lod, "Freeze terrain LOD");
                ui.checkbox(&mut self.volumes_enabled, "Volumetric terrain near the surface");
                ui.checkbox(&mut self.noclip, "Noclip (inspect underground matter)");
                if let Some(volume)=&self.volume {
                    ui.label(format!("Volume: {} bricks / {} triangles / 0.10 m finest", voxel::MAX_BRICKS,volume.triangles));
                    ui.label(format!("Volume generation {:.1} ms / {:.2} MiB geometry",volume.generation_ms,volume.bytes as f64/1048576.0));
                } else { ui.small("Volume: inactive or waiting for first mesh"); }
                ui.small(if self.volume_worker.busy {"Volume worker: generating (latest viewpoint)"} else {"Volume worker: idle"});
                ui.small(format!("Volume cache: {}/{} bricks",self.volume_worker.cached_bricks(),voxel::MAX_BRICKS));
                if let Some(error)=&self.volume_worker.error {ui.colored_label(egui::Color32::RED,error);}
                if ui.button("Fly just above this surface").clicked() {
                    self.release_input();
                    let d=self.camera.position.normalize();
                    self.camera.position=d*(self.planet.outer_surface_radius(d)+1.0);
                    self.camera.mode=CameraMode::Orbit;
                    self.camera.look_at_center();
                    self.toggle_flight();
                    self.camera.rotate(0.0,-520.0);
                }
                ui.small("GPU timing and cell inspection: pending");
                if ui.button("Toggle orbit / flight [Tab]").clicked() { self.toggle_flight(); }
                if ui.button("Reset to orbit [Home]").clicked() { self.release_input(); self.camera = Camera::new(config.radius); }
                ui.separator();
                ui.small("Orbit: LMB drag · wheel zoom\nFlight: mouse look · WASD move · Q/E roll left/right · Space/C up/down\nWheel adjusts speed · Shift faster · Ctrl slower\nEsc releases mouse · click scene to resume · Home resets");
            });
        });
        self.egui_input
            .handle_platform_output(&self.window, output.platform_output);
        if self.looking {
            self.window.set_cursor_visible(false);
        }
        let primitives = self.egui.tessellate(output.shapes, output.pixels_per_point);
        self.sphere
            .write_relative_vertices(&self.camera, &mut self.vertices);
        self.indices.clear();
        self.indices.extend_from_slice(&self.sphere.indices);
        self.batches.clear();
        self.batches.push(DrawBatch::macro_only(self.indices.len()));
        if self.volumes_enabled
            && let Some(volume) = &self.volume
        {
            volume.append(
                &self.camera,
                &mut self.vertices,
                &mut self.indices,
                &mut self.batches,
                self.volume_blend,
            );
        }
        let aspect = self.renderer.extent.width as f32 / self.renderer.extent.height as f32;
        if self.renderer.draw(
            &self.vertices,
            &self.indices,
            SceneDraw {
                matrix: self.camera.view_projection(aspect).to_cols_array(),
                batches: &self.batches,
            },
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
        let shortcut = matches!(&event, WindowEvent::KeyboardInput { event, .. }
            if matches!(event.physical_key, PhysicalKey::Code(key) if camera_shortcut(key)));
        let consumed = if running.looking || shortcut {
            false
        } else {
            running
                .egui_input
                .on_window_event(&running.window, &event)
                .consumed
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                running.renderer.needs_resize = true
            }
            WindowEvent::Focused(false) => running.release_input(),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key
                    && running.handle_key(key, event.state, event.repeat, consumed)
                {
                    event_loop.exit();
                }
            }
            WindowEvent::MouseInput { button, state, .. } => {
                let pressed = state == ElementState::Pressed;
                if button == MouseButton::Left && (!pressed || !consumed) {
                    if running.camera.mode == CameraMode::FreeFlight && pressed {
                        running.capture_mouse();
                    } else {
                        running.dragging = pressed;
                    }
                }
                if button == MouseButton::Right && pressed && !consumed {
                    running.capture_mouse();
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
            WindowEvent::MouseWheel { delta, .. } if !consumed => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                running.camera.zoom(lines);
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = running.frame(&self.config) {
                    self.fail(event_loop, error);
                } else if self.config.smoke_frames.is_some_and(|limit| {
                    running.frames >= limit
                        && running.smoke_surface_verified
                        && running.camera.altitude() > self.config.radius
                        && !running.renderer.needs_resize
                        && running.volume.is_none()
                        && !running.volume_worker.busy
                }) {
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
        {
            running.handle_mouse_motion(delta);
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
    if let Some(cell) = config.sample_cell {
        let planet = Planet {
            radius: config.radius,
            seed: config.seed,
        };
        let result = sample_effective_cell(&planet, &EmptyOverrideProvider, cell);
        anyhow::ensure!(
            result == sample_effective_cell(&planet, &EmptyOverrideProvider, cell),
            "Repeated matter query changed"
        );
        println!(
            "Planet {:?}\nCell ({}, {}, {})\nCentre {:?} m\nDensity {:+.12}\nMaterial {:?}\nSource {:?}\nRepeated query: identical",
            planet.identity(),
            cell.x,
            cell.y,
            cell.z,
            cell.center_meters().to_array(),
            result.sample.density,
            result.sample.material,
            result.source
        );
        return Ok(());
    }
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
    fn ui_consumption_cannot_block_camera_shortcuts() {
        for key in [KeyCode::Tab, KeyCode::Home, KeyCode::Escape] {
            assert!(camera_key_allowed(key, true));
            assert!(camera_key_allowed(key, false));
        }
        for key in [KeyCode::KeyW, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD] {
            assert!(!camera_key_allowed(key, true));
            assert!(camera_key_allowed(key, false));
        }
    }
    #[test]
    fn command_line_validates_radius_and_presets() {
        assert_eq!(parse(&[]).unwrap().unwrap().radius, EARTH_RADIUS_METERS);
        assert_eq!(parse(&["--seed", "42"]).unwrap().unwrap().seed, 42);
        assert_eq!(
            parse(&["--sample-cell", "-1", "2", "3"])
                .unwrap()
                .unwrap()
                .sample_cell,
            Some(CellCoord { x: -1, y: 2, z: 3 })
        );
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
            &["--seed", "-1"],
            &["--seed"],
            &["--sample-cell", "0", "0"],
            &["--sample-cell", "NaN", "0", "0"],
        ] {
            assert!(parse(args).is_err());
        }
    }
}
