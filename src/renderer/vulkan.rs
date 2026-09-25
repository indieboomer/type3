//! Explicit Vulkan ownership. One frame in flight keeps the foundation simple.
//! All resources are released before the device, including partial initialization.
use super::sphere::Vertex;
use anyhow::{Context, Result, anyhow, bail};
use ash::{Entry, vk};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::{ffi::CStr, io::Cursor};
use winit::window::Window;

struct ContextVk {
    _entry: Entry,
    instance: ash::Instance,
    surface_api: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    device: Option<ash::Device>,
    physical: vk::PhysicalDevice,
    queue: vk::Queue,
    family: u32,
    debug: Option<ash::ext::debug_utils::Instance>,
    messenger: vk::DebugUtilsMessengerEXT,
}

unsafe extern "system" fn debug_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _kind: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user: *mut std::ffi::c_void,
) -> vk::Bool32 {
    if !data.is_null() {
        let message = unsafe { CStr::from_ptr((*data).p_message) };
        eprintln!("Vulkan {severity:?}: {}", message.to_string_lossy());
    }
    vk::FALSE
}

impl ContextVk {
    fn new(window: &Window, validation: bool) -> Result<Self> {
        unsafe {
            let entry = Entry::load()
                .context("Vulkan loader unavailable; install a Vulkan-capable graphics driver")?;
            let mut extensions =
                ash_window::enumerate_required_extensions(window.display_handle()?.as_raw())?
                    .to_vec();
            let layer = c"VK_LAYER_KHRONOS_validation";
            let layers_available = entry.enumerate_instance_layer_properties()?;
            let validation_available = layers_available
                .iter()
                .any(|p| CStr::from_ptr(p.layer_name.as_ptr()) == layer);
            if validation && !validation_available {
                bail!(
                    "--validation requested, but VK_LAYER_KHRONOS_validation is not installed (install the Vulkan SDK)"
                );
            }
            let layers = if validation {
                vec![layer.as_ptr()]
            } else {
                vec![]
            };
            if validation {
                extensions.push(ash::ext::debug_utils::NAME.as_ptr());
            }
            let app = vk::ApplicationInfo::default()
                .application_name(c"Type3 matter browser")
                .api_version(vk::API_VERSION_1_1);
            let instance = entry.create_instance(
                &vk::InstanceCreateInfo::default()
                    .application_info(&app)
                    .enabled_extension_names(&extensions)
                    .enabled_layer_names(&layers),
                None,
            )?;
            let surface_api = ash::khr::surface::Instance::new(&entry, &instance);
            let mut context = Self {
                _entry: entry,
                instance,
                surface_api,
                surface: vk::SurfaceKHR::null(),
                device: None,
                physical: vk::PhysicalDevice::null(),
                queue: vk::Queue::null(),
                family: 0,
                debug: None,
                messenger: vk::DebugUtilsMessengerEXT::null(),
            };
            if validation {
                let api = ash::ext::debug_utils::Instance::new(&context._entry, &context.instance);
                context.messenger = api.create_debug_utils_messenger(
                    &vk::DebugUtilsMessengerCreateInfoEXT::default()
                        .message_severity(
                            vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                                | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
                        )
                        .message_type(
                            vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                                | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                                | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
                        )
                        .pfn_user_callback(Some(debug_callback)),
                    None,
                )?;
                context.debug = Some(api);
            }
            context.surface = ash_window::create_surface(
                &context._entry,
                &context.instance,
                window.display_handle()?.as_raw(),
                window.window_handle()?.as_raw(),
                None,
            )?;
            let mut candidates = Vec::new();
            for physical in context.instance.enumerate_physical_devices()? {
                let supported = context
                    .instance
                    .enumerate_device_extension_properties(physical)?
                    .iter()
                    .any(|ext| {
                        CStr::from_ptr(ext.extension_name.as_ptr()) == ash::khr::swapchain::NAME
                    });
                if !supported {
                    continue;
                }
                let families = context
                    .instance
                    .get_physical_device_queue_family_properties(physical);
                for (index, family) in families.iter().enumerate() {
                    if family.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                        && context.surface_api.get_physical_device_surface_support(
                            physical,
                            index as u32,
                            context.surface,
                        )?
                    {
                        let props = context.instance.get_physical_device_properties(physical);
                        let score = if props.device_type == vk::PhysicalDeviceType::DISCRETE_GPU {
                            2
                        } else {
                            1
                        };
                        candidates.push((score, physical, index as u32));
                        break;
                    }
                }
            }
            let (_, physical, family) = candidates
                .into_iter()
                .max_by_key(|c| c.0)
                .context("No Vulkan graphics device with presentation and swapchain support")?;
            context.physical = physical;
            context.family = family;
            let priorities = [1.0];
            let queues = [vk::DeviceQueueCreateInfo::default()
                .queue_family_index(family)
                .queue_priorities(&priorities)];
            let extensions = [ash::khr::swapchain::NAME.as_ptr()];
            let device = context.instance.create_device(
                physical,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queues)
                    .enabled_extension_names(&extensions),
                None,
            )?;
            context.queue = device.get_device_queue(family, 0);
            context.device = Some(device);
            Ok(context)
        }
    }

    fn device(&self) -> &ash::Device {
        self.device.as_ref().unwrap()
    }
    fn memory_type(&self, bits: u32, flags: vk::MemoryPropertyFlags) -> Result<u32> {
        let props = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical)
        };
        (0..props.memory_type_count)
            .find(|&i| {
                bits & (1 << i) != 0
                    && props.memory_types[i as usize]
                        .property_flags
                        .contains(flags)
            })
            .context("No suitable Vulkan memory type")
    }
}

impl Drop for ContextVk {
    fn drop(&mut self) {
        unsafe {
            if let Some(device) = &self.device {
                device.destroy_device(None);
            }
            self.surface_api.destroy_surface(self.surface, None);
            if let Some(debug) = &self.debug {
                debug.destroy_debug_utils_messenger(self.messenger, None);
            }
            self.instance.destroy_instance(None);
        }
    }
}

#[derive(Default)]
struct Buffer {
    handle: vk::Buffer,
    memory: vk::DeviceMemory,
    size: u64,
}

pub struct VulkanRenderer {
    context: ContextVk,
    swap_api: ash::khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    format: vk::SurfaceFormatKHR,
    pub extent: vk::Extent2D,
    views: Vec<vk::ImageView>,
    framebuffers: Vec<vk::Framebuffer>,
    depth: vk::Image,
    depth_memory: vk::DeviceMemory,
    depth_view: vk::ImageView,
    depth_format: vk::Format,
    render_pass: vk::RenderPass,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    pool: vk::CommandPool,
    command: vk::CommandBuffer,
    acquired: vk::Semaphore,
    presented: Vec<vk::Semaphore>,
    fence: vk::Fence,
    vertices: Buffer,
    indices: Buffer,
    index_count: u32,
    ui: Option<egui_ash_renderer::Renderer>,
    pub device_name: String,
    pub needs_resize: bool,
}

impl VulkanRenderer {
    pub fn new(
        window: &Window,
        vertex_count: usize,
        indices: &[u32],
        validation: bool,
    ) -> Result<Self> {
        let context = ContextVk::new(window, validation)?;
        let swap_api = ash::khr::swapchain::Device::new(&context.instance, context.device());
        let formats = unsafe {
            context
                .surface_api
                .get_physical_device_surface_formats(context.physical, context.surface)?
        };
        let format = formats
            .iter()
            .copied()
            .find(|f| {
                f.format == vk::Format::B8G8R8A8_SRGB
                    && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
            .or_else(|| {
                formats
                    .iter()
                    .copied()
                    .find(|f| f.format == vk::Format::R8G8B8A8_SRGB)
            })
            .context("Surface has no supported sRGB format")?;
        let depth_format = [vk::Format::D32_SFLOAT, vk::Format::D24_UNORM_S8_UINT]
            .into_iter()
            .find(|&f| {
                unsafe {
                    context
                        .instance
                        .get_physical_device_format_properties(context.physical, f)
                }
                .optimal_tiling_features
                .contains(vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT)
            })
            .context("No supported depth attachment format")?;
        let properties = unsafe {
            context
                .instance
                .get_physical_device_properties(context.physical)
        };
        let device_name = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let mut renderer = Self {
            context,
            swap_api,
            swapchain: vk::SwapchainKHR::null(),
            format,
            extent: vk::Extent2D::default(),
            views: vec![],
            framebuffers: vec![],
            depth: vk::Image::null(),
            depth_memory: vk::DeviceMemory::null(),
            depth_view: vk::ImageView::null(),
            depth_format,
            render_pass: vk::RenderPass::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            pool: vk::CommandPool::null(),
            command: vk::CommandBuffer::null(),
            acquired: vk::Semaphore::null(),
            presented: vec![],
            fence: vk::Fence::null(),
            vertices: Buffer::default(),
            indices: Buffer::default(),
            index_count: indices.len() as u32,
            ui: None,
            device_name,
            needs_resize: true,
        };
        unsafe {
            let device = renderer.context.device();
            renderer.pool = device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(renderer.context.family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?;
            renderer.command = device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(renderer.pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )?[0];
            renderer.acquired =
                device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?;
            renderer.fence = device.create_fence(
                &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                None,
            )?;
        }
        renderer.create_render_pass()?;
        renderer.create_pipeline()?;
        renderer.vertices = renderer.create_buffer(
            (vertex_count * size_of::<Vertex>()) as u64,
            vk::BufferUsageFlags::VERTEX_BUFFER,
        )?;
        renderer.indices = renderer.create_buffer(
            std::mem::size_of_val(indices) as u64,
            vk::BufferUsageFlags::INDEX_BUFFER,
        )?;
        renderer.upload(&renderer.indices, bytemuck::cast_slice(indices))?;
        renderer.ui = Some(egui_ash_renderer::Renderer::with_default_allocator(
            &renderer.context.instance,
            renderer.context.physical,
            renderer.context.device().clone(),
            renderer.render_pass,
            egui_ash_renderer::Options {
                in_flight_frames: 1,
                ..Default::default()
            },
        )?);
        renderer.resize(window)?;
        Ok(renderer)
    }

    fn create_buffer(&self, size: u64, usage: vk::BufferUsageFlags) -> Result<Buffer> {
        unsafe {
            let device = self.context.device();
            let handle = device.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(usage)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )?;
            let requirements = device.get_buffer_memory_requirements(handle);
            let result = (|| -> Result<vk::DeviceMemory> {
                let memory_type = self.context.memory_type(
                    requirements.memory_type_bits,
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                )?;
                let memory = device.allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(requirements.size)
                        .memory_type_index(memory_type),
                    None,
                )?;
                if let Err(error) = device.bind_buffer_memory(handle, memory, 0) {
                    device.free_memory(memory, None);
                    return Err(error.into());
                }
                Ok(memory)
            })();
            match result {
                Ok(memory) => Ok(Buffer {
                    handle,
                    memory,
                    size,
                }),
                Err(error) => {
                    device.destroy_buffer(handle, None);
                    Err(error)
                }
            }
        }
    }

    fn upload(&self, buffer: &Buffer, bytes: &[u8]) -> Result<()> {
        anyhow::ensure!(
            bytes.len() as u64 <= buffer.size,
            "GPU upload exceeds allocated buffer"
        );
        unsafe {
            let device = self.context.device();
            let mapped =
                device.map_memory(buffer.memory, 0, buffer.size, vk::MemoryMapFlags::empty())?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.cast(), bytes.len());
            device.unmap_memory(buffer.memory);
        }
        Ok(())
    }

    fn create_render_pass(&mut self) -> Result<()> {
        let attachments = [
            vk::AttachmentDescription::default()
                .format(self.format.format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::PRESENT_SRC_KHR),
            vk::AttachmentDescription::default()
                .format(self.depth_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL),
        ];
        let colors = [vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
        let depth = vk::AttachmentReference::default()
            .attachment(1)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        let subpasses = [vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&colors)
            .depth_stencil_attachment(&depth)];
        let stages = vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
            | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
            | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS;
        let dependencies = [vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(stages)
            .dst_stage_mask(stages)
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            )];
        self.render_pass = unsafe {
            self.context.device().create_render_pass(
                &vk::RenderPassCreateInfo::default()
                    .attachments(&attachments)
                    .subpasses(&subpasses)
                    .dependencies(&dependencies),
                None,
            )?
        };
        Ok(())
    }

    fn create_pipeline(&mut self) -> Result<()> {
        unsafe {
            let device = self.context.device();
            let vertex_code = ash::util::read_spv(&mut Cursor::new(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/vs_main.spv"
            ))))?;
            let fragment_code = ash::util::read_spv(&mut Cursor::new(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/fs_main.spv"
            ))))?;
            let vertex = device.create_shader_module(
                &vk::ShaderModuleCreateInfo::default().code(&vertex_code),
                None,
            )?;
            let fragment = match device.create_shader_module(
                &vk::ShaderModuleCreateInfo::default().code(&fragment_code),
                None,
            ) {
                Ok(module) => module,
                Err(error) => {
                    device.destroy_shader_module(vertex, None);
                    return Err(error.into());
                }
            };
            let result = (|| -> Result<()> {
                let ranges = [vk::PushConstantRange::default()
                    .stage_flags(vk::ShaderStageFlags::VERTEX)
                    .size(64)];
                self.pipeline_layout = device.create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&ranges),
                    None,
                )?;
                let stages = [
                    vk::PipelineShaderStageCreateInfo::default()
                        .stage(vk::ShaderStageFlags::VERTEX)
                        .module(vertex)
                        .name(c"vs_main"),
                    vk::PipelineShaderStageCreateInfo::default()
                        .stage(vk::ShaderStageFlags::FRAGMENT)
                        .module(fragment)
                        .name(c"fs_main"),
                ];
                let bindings = [vk::VertexInputBindingDescription {
                    binding: 0,
                    stride: size_of::<Vertex>() as u32,
                    input_rate: vk::VertexInputRate::VERTEX,
                }];
                let attributes = [
                    vk::VertexInputAttributeDescription {
                        location: 0,
                        binding: 0,
                        format: vk::Format::R32G32B32_SFLOAT,
                        offset: 0,
                    },
                    vk::VertexInputAttributeDescription {
                        location: 1,
                        binding: 0,
                        format: vk::Format::R32G32B32_SFLOAT,
                        offset: 12,
                    },
                ];
                let input = vk::PipelineVertexInputStateCreateInfo::default()
                    .vertex_binding_descriptions(&bindings)
                    .vertex_attribute_descriptions(&attributes);
                let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
                    .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
                let viewport = vk::PipelineViewportStateCreateInfo::default()
                    .viewport_count(1)
                    .scissor_count(1);
                let raster = vk::PipelineRasterizationStateCreateInfo::default()
                    .polygon_mode(vk::PolygonMode::FILL)
                    .cull_mode(vk::CullModeFlags::NONE)
                    .line_width(1.0);
                let multisample = vk::PipelineMultisampleStateCreateInfo::default()
                    .rasterization_samples(vk::SampleCountFlags::TYPE_1);
                let depth = vk::PipelineDepthStencilStateCreateInfo::default()
                    .depth_test_enable(true)
                    .depth_write_enable(true)
                    .depth_compare_op(vk::CompareOp::GREATER);
                let blend_attachment = [vk::PipelineColorBlendAttachmentState::default()
                    .color_write_mask(vk::ColorComponentFlags::RGBA)];
                let blend =
                    vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachment);
                let dynamics = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
                let dynamic =
                    vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamics);
                let info = vk::GraphicsPipelineCreateInfo::default()
                    .stages(&stages)
                    .vertex_input_state(&input)
                    .input_assembly_state(&assembly)
                    .viewport_state(&viewport)
                    .rasterization_state(&raster)
                    .multisample_state(&multisample)
                    .depth_stencil_state(&depth)
                    .color_blend_state(&blend)
                    .dynamic_state(&dynamic)
                    .layout(self.pipeline_layout)
                    .render_pass(self.render_pass)
                    .subpass(0);
                self.pipeline = device
                    .create_graphics_pipelines(vk::PipelineCache::null(), &[info], None)
                    .map_err(|(pipelines, error)| {
                        for p in pipelines {
                            device.destroy_pipeline(p, None);
                        }
                        anyhow!("Create sphere pipeline: {error:?}")
                    })?[0];
                Ok(())
            })();
            device.destroy_shader_module(fragment, None);
            device.destroy_shader_module(vertex, None);
            result
        }
    }

    pub fn resize(&mut self, window: &Window) -> Result<()> {
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        unsafe {
            self.context.device().device_wait_idle()?;
            self.destroy_swapchain();
            let caps = self
                .context
                .surface_api
                .get_physical_device_surface_capabilities(
                    self.context.physical,
                    self.context.surface,
                )?;
            self.extent = if caps.current_extent.width != u32::MAX {
                caps.current_extent
            } else {
                vk::Extent2D {
                    width: size
                        .width
                        .clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                    height: size
                        .height
                        .clamp(caps.min_image_extent.height, caps.max_image_extent.height),
                }
            };
            let count = if caps.max_image_count == 0 {
                caps.min_image_count + 1
            } else {
                (caps.min_image_count + 1).min(caps.max_image_count)
            };
            let alpha = [
                vk::CompositeAlphaFlagsKHR::OPAQUE,
                vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
                vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
                vk::CompositeAlphaFlagsKHR::INHERIT,
            ]
            .into_iter()
            .find(|f| caps.supported_composite_alpha.contains(*f))
            .context("No composite alpha mode")?;
            self.swapchain = self.swap_api.create_swapchain(
                &vk::SwapchainCreateInfoKHR::default()
                    .surface(self.context.surface)
                    .min_image_count(count)
                    .image_format(self.format.format)
                    .image_color_space(self.format.color_space)
                    .image_extent(self.extent)
                    .image_array_layers(1)
                    .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
                    .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .pre_transform(caps.current_transform)
                    .composite_alpha(alpha)
                    .present_mode(vk::PresentModeKHR::FIFO)
                    .clipped(true),
                None,
            )?;
            let device = self.context.device();
            for image in self.swap_api.get_swapchain_images(self.swapchain)? {
                self.views.push(
                    device.create_image_view(
                        &vk::ImageViewCreateInfo::default()
                            .image(image)
                            .view_type(vk::ImageViewType::TYPE_2D)
                            .format(self.format.format)
                            .subresource_range(
                                vk::ImageSubresourceRange::default()
                                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                                    .level_count(1)
                                    .layer_count(1),
                            ),
                        None,
                    )?,
                );
                self.presented
                    .push(device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?);
            }
            self.depth = device.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(self.depth_format)
                    .extent(vk::Extent3D {
                        width: self.extent.width,
                        height: self.extent.height,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )?;
            let req = device.get_image_memory_requirements(self.depth);
            let memory_type = self
                .context
                .memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)?;
            self.depth_memory = device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(memory_type),
                None,
            )?;
            device.bind_image_memory(self.depth, self.depth_memory, 0)?;
            self.depth_view = device.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(self.depth)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(self.depth_format)
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(vk::ImageAspectFlags::DEPTH)
                            .level_count(1)
                            .layer_count(1),
                    ),
                None,
            )?;
            for &view in &self.views {
                let attachments = [view, self.depth_view];
                self.framebuffers.push(
                    device.create_framebuffer(
                        &vk::FramebufferCreateInfo::default()
                            .render_pass(self.render_pass)
                            .attachments(&attachments)
                            .width(self.extent.width)
                            .height(self.extent.height)
                            .layers(1),
                        None,
                    )?,
                );
            }
            self.needs_resize = false;
        }
        Ok(())
    }

    pub fn draw(
        &mut self,
        vertices: &[Vertex],
        matrix: [f32; 16],
        primitives: &[egui::ClippedPrimitive],
        textures: &egui::TexturesDelta,
        pixels_per_point: f32,
    ) -> Result<bool> {
        unsafe {
            let device = self.context.device();
            device.wait_for_fences(&[self.fence], true, u64::MAX)?;
            // Texture updates must survive an out-of-date acquire (not be lost with the UI frame).
            self.ui
                .as_mut()
                .unwrap()
                .set_textures(self.context.queue, self.pool, &textures.set)?;
            let (image, suboptimal) = match self.swap_api.acquire_next_image(
                self.swapchain,
                u64::MAX,
                self.acquired,
                vk::Fence::null(),
            ) {
                Ok(result) => result,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    self.needs_resize = true;
                    self.ui.as_mut().unwrap().free_textures(&textures.free)?;
                    return Ok(false);
                }
                Err(error) => return Err(error.into()),
            };
            self.needs_resize |= suboptimal;
            self.upload(&self.vertices, bytemuck::cast_slice(vertices))?;
            device.reset_command_buffer(self.command, vk::CommandBufferResetFlags::empty())?;
            device.begin_command_buffer(
                self.command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            let clears = [
                vk::ClearValue {
                    color: vk::ClearColorValue {
                        float32: [0.002, 0.003, 0.008, 1.0],
                    },
                },
                vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue {
                        depth: 0.0,
                        stencil: 0,
                    },
                },
            ];
            let area = vk::Rect2D {
                offset: vk::Offset2D::default(),
                extent: self.extent,
            };
            device.cmd_begin_render_pass(
                self.command,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(self.render_pass)
                    .framebuffer(self.framebuffers[image as usize])
                    .render_area(area)
                    .clear_values(&clears),
                vk::SubpassContents::INLINE,
            );
            device.cmd_bind_pipeline(self.command, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            device.cmd_set_viewport(
                self.command,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: self.extent.width as f32,
                    height: self.extent.height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
            device.cmd_set_scissor(self.command, 0, &[area]);
            device.cmd_bind_vertex_buffers(self.command, 0, &[self.vertices.handle], &[0]);
            device.cmd_bind_index_buffer(
                self.command,
                self.indices.handle,
                0,
                vk::IndexType::UINT32,
            );
            device.cmd_push_constants(
                self.command,
                self.pipeline_layout,
                vk::ShaderStageFlags::VERTEX,
                0,
                bytemuck::cast_slice(&matrix),
            );
            device.cmd_draw_indexed(self.command, self.index_count, 1, 0, 0, 0);
            self.ui.as_mut().unwrap().cmd_draw(
                self.command,
                self.extent,
                pixels_per_point,
                primitives,
            )?;
            device.cmd_end_render_pass(self.command);
            device.end_command_buffer(self.command)?;
            let wait = [self.acquired];
            let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
            let commands = [self.command];
            let signal = [self.presented[image as usize]];
            device.reset_fences(&[self.fence])?;
            device.queue_submit(
                self.context.queue,
                &[vk::SubmitInfo::default()
                    .wait_semaphores(&wait)
                    .wait_dst_stage_mask(&wait_stages)
                    .command_buffers(&commands)
                    .signal_semaphores(&signal)],
                self.fence,
            )?;
            let chains = [self.swapchain];
            let images = [image];
            match self.swap_api.queue_present(
                self.context.queue,
                &vk::PresentInfoKHR::default()
                    .wait_semaphores(&signal)
                    .swapchains(&chains)
                    .image_indices(&images),
            ) {
                Ok(suboptimal) => self.needs_resize |= suboptimal,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => self.needs_resize = true,
                Err(error) => return Err(error.into()),
            }
            if !textures.free.is_empty() {
                device.wait_for_fences(&[self.fence], true, u64::MAX)?;
                self.ui.as_mut().unwrap().free_textures(&textures.free)?;
            }
        }
        Ok(true)
    }

    fn destroy_swapchain(&mut self) {
        unsafe {
            let device = self.context.device();
            for frame in self.framebuffers.drain(..) {
                device.destroy_framebuffer(frame, None);
            }
            device.destroy_image_view(self.depth_view, None);
            device.destroy_image(self.depth, None);
            device.free_memory(self.depth_memory, None);
            self.depth_view = vk::ImageView::null();
            self.depth = vk::Image::null();
            self.depth_memory = vk::DeviceMemory::null();
            for view in self.views.drain(..) {
                device.destroy_image_view(view, None);
            }
            for semaphore in self.presented.drain(..) {
                device.destroy_semaphore(semaphore, None);
            }
            self.swap_api.destroy_swapchain(self.swapchain, None);
            self.swapchain = vk::SwapchainKHR::null();
        }
    }
}

impl Drop for VulkanRenderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.context.device().device_wait_idle();
            self.ui.take();
            self.destroy_swapchain();
            let device = self.context.device();
            for buffer in [&self.vertices, &self.indices] {
                device.destroy_buffer(buffer.handle, None);
                device.free_memory(buffer.memory, None);
            }
            device.destroy_pipeline(self.pipeline, None);
            device.destroy_pipeline_layout(self.pipeline_layout, None);
            device.destroy_render_pass(self.render_pass, None);
            device.destroy_semaphore(self.acquired, None);
            device.destroy_fence(self.fence, None);
            device.destroy_command_pool(self.pool, None);
        }
    }
}
