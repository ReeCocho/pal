use api::{
    buffer::{BufferCreateError, BufferCreateInfo, BufferViewError},
    command_buffer::Command,
    descriptor_set::{
        DescriptorSetCreateError, DescriptorSetCreateInfo, DescriptorSetLayoutCreateError,
        DescriptorSetLayoutCreateInfo,
    },
    graphics_pipeline::{GraphicsPipelineCreateError, GraphicsPipelineCreateInfo},
    queue::SurfacePresentFailure,
    render_pass::{ColorAttachmentSource, RenderPassDescriptor},
    shader::{ShaderCreateError, ShaderCreateInfo},
    surface::{
        SurfaceConfiguration, SurfaceCreateError, SurfaceCreateInfo, SurfaceImageAcquireError,
        SurfacePresentSuccess, SurfaceUpdateError,
    },
    types::*,
    Backend,
};
use ash::vk::{self, DebugUtilsMessageSeverityFlagsEXT};
use buffer::Buffer;
use crossbeam_utils::sync::ShardedLock;
use gpu_allocator::vulkan::*;
use graphics_pipeline::GraphicsPipeline;
use queue::VkQueue;
use raw_window_handle::HasRawWindowHandle;
use render_pass::{FramebufferCache, RenderPassCache};
use shader::Shader;
use std::{
    borrow::Cow,
    collections::HashSet,
    ffi::{CStr, CString},
    mem::ManuallyDrop,
    ptr::NonNull,
    sync::Mutex,
};
use surface::{Surface, SurfaceImage};
use thiserror::Error;
use util::{
    garbage_collector::{GarbageCollector, TimelineValues},
    pipeline_tracker::PipelineTracker,
    resource_state::{LatestUsage, ResourceState},
    semaphores::{SemaphoreTracker, WaitInfo},
};

use crate::util::pipeline_tracker::ResourceUsage;

pub mod buffer;
pub mod command_buffer;
pub mod graphics_pipeline;
pub mod queue;
pub mod render_pass;
pub mod shader;
pub mod surface;
pub mod util;

pub struct VulkanBackendCreateInfo<'a, W: HasRawWindowHandle> {
    pub app_name: String,
    pub engine_name: String,
    /// A window is required to find a queue that supports presentation.
    pub window: &'a W,
    /// Enables debugging layers and extensions.
    pub debug: bool,
}

#[derive(Debug, Error)]
pub enum VulkanBackendCreateError {
    #[error("vulkan error: {0}")]
    Vulkan(vk::Result),
    #[error("ash load error: {0}")]
    AshLoadError(ash::LoadingError),
    #[error("no suitable graphics device was found")]
    NoDevice,
}

pub struct VulkanBackend {
    pub(crate) entry: ash::Entry,
    pub(crate) instance: ash::Instance,
    pub(crate) debug: Option<(ash::extensions::ext::DebugUtils, vk::DebugUtilsMessengerEXT)>,
    pub(crate) physical_device: vk::PhysicalDevice,
    pub(crate) queue_family_indices: QueueFamilyIndices,
    pub(crate) properties: vk::PhysicalDeviceProperties,
    pub(crate) features: vk::PhysicalDeviceFeatures,
    pub(crate) device: ash::Device,
    pub(crate) surface_loader: ash::extensions::khr::Surface,
    pub(crate) swapchain_loader: ash::extensions::khr::Swapchain,
    pub(crate) main: ShardedLock<VkQueue>,
    pub(crate) transfer: ShardedLock<VkQueue>,
    pub(crate) present: ShardedLock<VkQueue>,
    pub(crate) compute: ShardedLock<VkQueue>,
    pub(crate) allocator: ManuallyDrop<Mutex<Allocator>>,
    pub(crate) render_passes: RenderPassCache,
    pub(crate) framebuffers: FramebufferCache,
    pub(crate) garbage: GarbageCollector,
    pub(crate) resource_state: ShardedLock<ResourceState>,
}

#[derive(Default)]
pub(crate) struct QueueFamilyIndices {
    /// Must support graphics, transfer, and compute.
    pub main: u32,
    /// Must support presentation.
    pub present: u32,
    /// Must support transfer.
    pub transfer: u32,
    /// Must support compute.
    pub compute: u32,
    /// Contains all queue families which are unique (some queue families may be equivilent on
    /// certain hardware.
    pub unique: Vec<u32>,
}

struct PhysicalDeviceQuery {
    pub device: vk::PhysicalDevice,
    pub queue_family_indices: QueueFamilyIndices,
    pub properties: vk::PhysicalDeviceProperties,
    pub features: vk::PhysicalDeviceFeatures,
}

impl Backend for VulkanBackend {
    type Buffer = Buffer;
    type Texture = ();
    type Surface = Surface;
    type SurfaceImage = SurfaceImage;
    type Shader = Shader;
    type GraphicsPipeline = GraphicsPipeline;
    type DescriptorSetLayout = ();
    type DescriptorSet = ();
    type Job = ();

    unsafe fn create_surface<'a, W: HasRawWindowHandle>(
        &self,
        create_info: SurfaceCreateInfo<'a, W>,
    ) -> Result<Self::Surface, SurfaceCreateError> {
        // Create the surface
        let surface =
            match ash_window::create_surface(&self.entry, &self.instance, create_info.window, None)
            {
                Ok(surface) => surface,
                Err(err) => return Err(SurfaceCreateError::Other(err.to_string())),
            };

        let mut surface = Surface {
            surface,
            swapchain: vk::SwapchainKHR::null(),
            format: vk::SurfaceFormatKHR::default(),
            resolution: vk::Extent2D::default(),
            images: Vec::default(),
            semaphores: Vec::default(),
            next_semaphore: 0,
            images_acquired: 0,
        };

        // Update the surface with the provided configuration
        if let Err(err) = surface.update_config(self, create_info.config) {
            return Err(SurfaceCreateError::BadConfig(err));
        }

        Ok(surface)
    }

    #[inline(always)]
    unsafe fn destroy_surface(&self, surface: &mut Self::Surface) {
        self.device.device_wait_idle().unwrap();
        surface.release(self);
        self.surface_loader.destroy_surface(surface.surface, None);
    }

    #[inline(always)]
    unsafe fn surface_dimensions(&self, surface: &Self::Surface) -> (u32, u32) {
        surface.dimensions()
    }

    #[inline(always)]
    unsafe fn update_surface(
        &self,
        surface: &mut Self::Surface,
        config: SurfaceConfiguration,
    ) -> Result<(), SurfaceUpdateError> {
        self.device.device_wait_idle().unwrap();

        // Signal that the views are about to be destroyed
        for (_, view) in &surface.images {
            self.framebuffers.view_destroyed(&self.device, *view);
        }

        // Then update the config
        surface.update_config(self, config)
    }

    #[inline(always)]
    unsafe fn acquire_image(
        &self,
        surface: &mut Self::Surface,
    ) -> Result<Self::SurfaceImage, SurfaceImageAcquireError> {
        surface.acquire_image(self)
    }

    unsafe fn present_image(
        &self,
        surface: &Self::Surface,
        image: &mut Self::SurfaceImage,
    ) -> Result<SurfacePresentSuccess, SurfacePresentFailure> {
        if image.surface() != surface.surface {
            return Err(SurfacePresentFailure::BadImage);
        }

        if !image.is_signaled() {
            return Err(SurfacePresentFailure::NoRender);
        }

        // Present
        let invalidated = {
            let idx = [image.index() as u32];
            let swapchain = [surface.swapchain];
            let presentable = [image.semaphores().presentable];
            let present_info = vk::PresentInfoKHR::builder()
                .image_indices(&idx)
                .swapchains(&swapchain)
                .wait_semaphores(&presentable)
                .build();
            self.swapchain_loader
                .queue_present(self.present.try_read().unwrap().queue, &present_info)
                .unwrap_or(true)
        };

        if invalidated {
            Ok(SurfacePresentSuccess::Invalidated)
        } else {
            Ok(SurfacePresentSuccess::Ok)
        }
    }

    unsafe fn destroy_surface_image(&self, image: &mut Self::SurfaceImage) {
        if !image.is_signaled() {
            todo!()
        }
    }

    unsafe fn submit_commands<'a>(&self, queue: QueueType, commands: Vec<Command<'a, Self>>) {
        // Lock down all neccesary objects
        let mut resc_state = self.resource_state.write().unwrap();
        let mut allocator = self.allocator.lock().unwrap();
        let mut main = self.main.write().unwrap();
        let mut transfer = self.transfer.write().unwrap();
        let mut compute = self.compute.write().unwrap();
        let mut present = self.present.write().unwrap();

        // Perform garbage collection
        let current_values = TimelineValues {
            main: main.current_timeline_value(&self.device),
            transfer: transfer.current_timeline_value(&self.device),
            compute: compute.current_timeline_value(&self.device),
        };
        let target_values = TimelineValues {
            main: main.target_timeline_value(),
            transfer: transfer.target_timeline_value(),
            compute: compute.target_timeline_value(),
        };
        self.garbage
            .cleanup(&self.device, &mut allocator, current_values, target_values);

        // State
        let mut semaphore_tracker = SemaphoreTracker::default();
        let mut active_render_pass = vk::RenderPass::null();
        let mut pipeline_tracker = PipelineTracker::default();
        let next_target_value = match queue {
            QueueType::Main => &main,
            QueueType::Transfer => &transfer,
            QueueType::Compute => &compute,
            QueueType::Present => &present,
        }
        .target_timeline_value()
            + 1;

        // Acquire a command buffer from the queue
        let cb = match queue {
            QueueType::Main => &mut main,
            QueueType::Transfer => &mut transfer,
            QueueType::Compute => &mut compute,
            QueueType::Present => &mut present,
        }
        .allocate_command_buffer(&self.device);
        let begin_info = vk::CommandBufferBeginInfo::builder()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
            .build();
        self.device.begin_command_buffer(cb, &begin_info).unwrap();

        // Interpret commands
        for command in &commands {
            match command {
                Command::BeginRenderPass(descriptor) => {
                    // Barrier check
                    find_render_pass_resources(
                        &descriptor,
                        &commands,
                        &self.device,
                        cb,
                        &mut pipeline_tracker,
                    );

                    // Get the render pass described
                    active_render_pass = self.render_passes.get(&self.device, &descriptor);

                    // Find the render pass
                    let mut dims = (0, 0);
                    let mut views = Vec::with_capacity(descriptor.color_attachments.len());
                    for attachment in &descriptor.color_attachments {
                        views.push(match &attachment.source {
                            ColorAttachmentSource::SurfaceImage(image) => {
                                // Indicate that the surface image has been drawn to
                                image.internal().signal_draw();

                                // Grab semaphores
                                let semaphores = image.internal().semaphores();
                                semaphore_tracker.register_signal(semaphores.presentable, None);
                                semaphore_tracker.register_wait(
                                    semaphores.available,
                                    WaitInfo {
                                        value: None,
                                        stage: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                                    },
                                );

                                dims = image.internal().dims();
                                image.internal().view()
                            }
                        });
                    }

                    // Find the framebuffer
                    let framebuffer = self.framebuffers.get(
                        &self.device,
                        active_render_pass,
                        views,
                        vk::Extent2D {
                            width: dims.0,
                            height: dims.1,
                        },
                    );

                    // Find clear values
                    let mut clear_values = Vec::with_capacity(descriptor.color_attachments.len());
                    for attachment in &descriptor.color_attachments {
                        if let LoadOp::Clear(clear_color) = &attachment.load_op {
                            let color = match clear_color {
                                ClearColor::RgbaF32(r, g, b, a) => vk::ClearColorValue {
                                    float32: [*r, *g, *b, *a],
                                },
                                ClearColor::RU32(r) => vk::ClearColorValue {
                                    uint32: [*r, 0, 0, 0],
                                },
                                ClearColor::D32S32(_, _) => panic!("invalid clear color type"),
                            };
                            clear_values.push(vk::ClearValue { color });
                        }
                    }

                    // Initial viewport configuration
                    // NOTE: Viewport is flipped to account for Vulkan coordinate system
                    let viewport = [vk::Viewport {
                        width: dims.0 as f32,
                        height: -(dims.1 as f32),
                        x: 0.0,
                        y: dims.1 as f32,
                        min_depth: 0.0,
                        max_depth: 1.0,
                    }];

                    let scissor = [vk::Rect2D {
                        extent: vk::Extent2D {
                            width: dims.0,
                            height: dims.1,
                        },
                        offset: vk::Offset2D { x: 0, y: 0 },
                    }];

                    self.device.cmd_set_viewport(cb, 0, &viewport);
                    self.device.cmd_set_scissor(cb, 0, &scissor);

                    // Begin the render pass
                    let begin_info = vk::RenderPassBeginInfo::builder()
                        .render_pass(active_render_pass)
                        .clear_values(&clear_values)
                        .framebuffer(framebuffer)
                        .render_area(vk::Rect2D {
                            offset: vk::Offset2D { x: 0, y: 0 },
                            extent: vk::Extent2D {
                                width: dims.0,
                                height: dims.1,
                            },
                        })
                        .build();

                    self.device
                        .cmd_begin_render_pass(cb, &begin_info, vk::SubpassContents::INLINE);
                }
                Command::EndRenderPass => self.device.cmd_end_render_pass(cb),
                Command::BindGraphicsPipeline(pipeline) => {
                    let pipeline = pipeline.internal().get(&self.device, active_render_pass);
                    self.device
                        .cmd_bind_pipeline(cb, vk::PipelineBindPoint::GRAPHICS, pipeline);
                }
                Command::BindVertexBuffers { first, binds } => {
                    let mut buffers = Vec::with_capacity(binds.len());
                    let mut offsets = Vec::with_capacity(binds.len());
                    for bind in binds {
                        let buffer = bind.buffer.internal();

                        // Semaphore check
                        if let Some(old) = resc_state.set_buffer(
                            buffer.buffer,
                            bind.array_element,
                            Some(LatestUsage {
                                queue,
                                value: next_target_value,
                            }),
                        ) {
                            old.wait_if_needed(
                                &mut semaphore_tracker,
                                queue,
                                vk::PipelineStageFlags::VERTEX_INPUT,
                                &main,
                                &transfer,
                                &compute,
                                &present,
                            );
                        }

                        buffers.push(buffer.buffer);
                        offsets
                            .push((buffer.aligned_size * bind.array_element as u64) + bind.offset);
                    }
                    self.device
                        .cmd_bind_vertex_buffers(cb, *first as u32, &buffers, &offsets);
                }
                Command::BindIndexBuffer {
                    buffer,
                    array_element,
                    offset,
                    ty,
                } => {
                    let buffer = buffer.internal();

                    // Semaphore check
                    if let Some(old) = resc_state.set_buffer(
                        buffer.buffer,
                        *array_element,
                        Some(LatestUsage {
                            queue,
                            value: next_target_value,
                        }),
                    ) {
                        old.wait_if_needed(
                            &mut semaphore_tracker,
                            queue,
                            vk::PipelineStageFlags::VERTEX_INPUT,
                            &main,
                            &transfer,
                            &compute,
                            &present,
                        );
                    }

                    self.device.cmd_bind_index_buffer(
                        cb,
                        buffer.buffer,
                        (buffer.aligned_size * *array_element as u64) + offset,
                        crate::util::to_vk_index_type(*ty),
                    );
                }
                Command::DrawIndexed {
                    index_count,
                    instance_count,
                    first_index,
                    vertex_offset,
                    first_instance,
                } => {
                    self.device.cmd_draw_indexed(
                        cb,
                        *index_count as u32,
                        *instance_count as u32,
                        *first_index as u32,
                        *vertex_offset as i32,
                        *first_instance as u32,
                    );
                }
                Command::CopyBufferToBuffer(copy) => {
                    // Barrier check
                    let src = copy.src.internal();
                    let dst = copy.dst.internal();
                    if let Some(barrier) = pipeline_tracker.update(
                        &[
                            (
                                src.buffer,
                                ResourceUsage {
                                    used: vk::PipelineStageFlags::TRANSFER,
                                    access: vk::AccessFlags::TRANSFER_READ,
                                },
                            ),
                            (
                                dst.buffer,
                                ResourceUsage {
                                    used: vk::PipelineStageFlags::TRANSFER,
                                    access: vk::AccessFlags::TRANSFER_WRITE,
                                },
                            ),
                        ],
                        &[],
                    ) {
                        barrier.execute(&self.device, cb);
                    }

                    // Semaphore check
                    if let Some(old) = resc_state.set_buffer(
                        src.buffer,
                        copy.src_array_element,
                        Some(LatestUsage {
                            queue,
                            value: next_target_value,
                        }),
                    ) {
                        old.wait_if_needed(
                            &mut semaphore_tracker,
                            queue,
                            vk::PipelineStageFlags::TRANSFER,
                            &main,
                            &transfer,
                            &compute,
                            &present,
                        );
                    }

                    if let Some(old) = resc_state.set_buffer(
                        dst.buffer,
                        copy.dst_array_element,
                        Some(LatestUsage {
                            queue,
                            value: next_target_value,
                        }),
                    ) {
                        old.wait_if_needed(
                            &mut semaphore_tracker,
                            queue,
                            vk::PipelineStageFlags::TRANSFER,
                            &main,
                            &transfer,
                            &compute,
                            &present,
                        );
                    }

                    // Perform copy
                    let region = [vk::BufferCopy::builder()
                        .dst_offset(
                            (dst.aligned_size * copy.dst_array_element as u64) + copy.dst_offset,
                        )
                        .src_offset(
                            (src.aligned_size * copy.src_array_element as u64) + copy.src_offset,
                        )
                        .size(copy.len)
                        .build()];
                    self.device
                        .cmd_copy_buffer(cb, src.buffer, dst.buffer, &region);
                }
            }
        }

        // Submit to the queue
        self.device.end_command_buffer(cb).unwrap();
        match queue {
            QueueType::Main => main,
            QueueType::Transfer => transfer,
            QueueType::Compute => compute,
            QueueType::Present => present,
        }
        .submit(&self.device, cb, semaphore_tracker)
        .unwrap();
    }

    unsafe fn synchronize_queue(&self, queue: api::types::QueueType) {
        todo!()
    }

    unsafe fn wait_on(
        &self,
        job: &Self::Job,
        timeout: Option<std::time::Duration>,
    ) -> api::types::JobStatus {
        todo!()
    }

    unsafe fn poll_status(&self, job: &Self::Job) -> api::types::JobStatus {
        todo!()
    }

    unsafe fn create_buffer(
        &self,
        create_info: BufferCreateInfo,
    ) -> Result<Self::Buffer, BufferCreateError> {
        Buffer::new(
            &self.device,
            self.garbage.sender(),
            &mut self.allocator.lock().unwrap(),
            &self.properties.limits,
            create_info,
        )
    }

    unsafe fn create_texture(
        &self,
        create_info: api::texture::TextureCreateInfo<Self>,
    ) -> Self::Texture {
        todo!()
    }

    unsafe fn create_shader(
        &self,
        create_info: ShaderCreateInfo,
    ) -> Result<Self::Shader, ShaderCreateError> {
        let code = match bytemuck::try_cast_slice::<u8, u32>(create_info.code) {
            Ok(code) => code,
            Err(_) => {
                return Err(ShaderCreateError::Other(String::from(
                    "shader code size is not a multiple of 4",
                )))
            }
        };
        let create_info = vk::ShaderModuleCreateInfo::builder().code(code).build();
        let module = match self.device.create_shader_module(&create_info, None) {
            Ok(module) => module,
            Err(err) => return Err(ShaderCreateError::Other(err.to_string())),
        };
        Ok(Shader { module })
    }

    unsafe fn create_graphics_pipeline(
        &self,
        create_info: GraphicsPipelineCreateInfo<Self>,
    ) -> Result<Self::GraphicsPipeline, GraphicsPipelineCreateError> {
        Ok(GraphicsPipeline::new(
            &self.device,
            self.garbage.sender(),
            create_info,
        ))
    }

    unsafe fn create_descriptor_set(
        &self,
        create_info: DescriptorSetCreateInfo<Self>,
    ) -> Result<Self::DescriptorSet, DescriptorSetCreateError> {
        todo!()
    }

    unsafe fn create_descriptor_set_layout(
        &self,
        create_info: DescriptorSetLayoutCreateInfo<Self>,
    ) -> Result<Self::DescriptorSetLayout, DescriptorSetLayoutCreateError> {
        todo!()
    }

    unsafe fn destroy_buffer(&self, _buffer: &mut Self::Buffer) {
        // Handled in drop
    }

    unsafe fn destroy_texture(&self, id: &mut Self::Texture) {
        todo!()
    }

    unsafe fn destroy_shader(&self, shader: &mut Self::Shader) {
        self.device.destroy_shader_module(shader.module, None);
    }

    unsafe fn destroy_graphics_pipeline(&self, _pipeline: &mut Self::GraphicsPipeline) {
        // Handled in drop
    }

    unsafe fn destroy_descriptor_set(&self, id: &mut Self::DescriptorSet) {
        todo!()
    }

    unsafe fn destroy_descriptor_set_layout(&self, id: &mut Self::DescriptorSetLayout) {
        todo!()
    }

    unsafe fn map_memory(
        &self,
        id: &mut Self::Buffer,
        idx: usize,
    ) -> Result<(NonNull<u8>, u64), BufferViewError> {
        // Wait until the last queue that the buffer was used in has finished it's work
        let mut resc_state = self.resource_state.write().unwrap();

        // NOTE: The reason we set the usage to `None` is because we have to wait for the previous
        // usage to complete. This implies that no one is using this buffer anymore and thus no
        // waits are needed further.
        if let Some(old) = resc_state.set_buffer(id.buffer, idx, None) {
            let queue = match old.queue {
                QueueType::Main => self.main.read().unwrap(),
                QueueType::Transfer => self.transfer.read().unwrap(),
                QueueType::Compute => self.compute.read().unwrap(),
                QueueType::Present => self.present.read().unwrap(),
            };

            // If the queue is up to speed with previous usage, we can just wait using the API wait
            if queue.target_timeline_value() == old.value {
                let semaphore = [queue.semaphore()];
                let value = [old.value];
                let wait = vk::SemaphoreWaitInfo::builder()
                    .semaphores(&semaphore)
                    .values(&value)
                    .build();
                self.device.wait_semaphores(&wait, u64::MAX).unwrap();
            }
            // Otherwise, we have to spin since the timeline value might overshoot the value we
            // actually want to wait on.
            else {
                let semaphore = queue.semaphore();
                while self.device.get_semaphore_counter_value(semaphore).unwrap() < old.value {
                    std::hint::spin_loop();
                }
            }
        }

        let map = id.block.mapped_ptr().unwrap();
        let map =
            NonNull::new_unchecked((map.as_ptr() as *mut u8).add(id.aligned_size as usize * idx));
        Ok((map, id.size))
    }

    unsafe fn unmap_memory(&self, _id: &mut Self::Buffer) {
        // Handled by the allocator
    }

    unsafe fn flush_range(&self, _id: &mut Self::Buffer, _idx: usize) {
        // Not needed because HOST_COHERENT
    }

    unsafe fn invalidate_range(&self, _id: &mut Self::Buffer, _idx: usize) {
        // Not needed because HOST_COHERENT
    }

    unsafe fn update_descriptor_sets(&self) {
        todo!()
    }
}

impl VulkanBackend {
    pub fn new<'a, W: HasRawWindowHandle>(
        create_info: VulkanBackendCreateInfo<'a, W>,
    ) -> Result<Self, VulkanBackendCreateError> {
        let app_name = CString::new(create_info.app_name).unwrap();
        let vk_version = vk::API_VERSION_1_2;

        // Get required instance layers
        let layer_names = if create_info.debug {
            vec![
                CStr::from_bytes_with_nul(b"VK_LAYER_KHRONOS_validation\0").unwrap(),
                CStr::from_bytes_with_nul(b"VK_LAYER_KHRONOS_synchronization2\0").unwrap(),
            ]
            .into_iter()
            .map(|r| r.as_ptr())
            .collect::<Vec<_>>()
        } else {
            Vec::default()
        };

        // Get required instance extensions
        let instance_extensions = {
            let mut extensions = ash_window::enumerate_required_extensions(create_info.window)?
                .iter()
                .map(|ext| unsafe { CStr::from_ptr(*ext) })
                .collect::<Vec<_>>();

            if create_info.debug {
                extensions.push(ash::extensions::ext::DebugUtils::name());
            }

            extensions
                .into_iter()
                .map(|r| r.as_ptr())
                .collect::<Vec<_>>()
        };

        // Get required device extensions
        let device_extensions = {
            let mut extensions = vec![
                ash::extensions::khr::Swapchain::name(),
                ash::extensions::khr::TimelineSemaphore::name(),
            ];
            if create_info.debug {
                extensions
                    .push(CStr::from_bytes_with_nul(b"VK_KHR_shader_non_semantic_info\0").unwrap())
            }
            extensions
                .into_iter()
                .map(|r| r.as_ptr())
                .collect::<Vec<_>>()
        };

        // Dynamically load Vulkan
        let entry = unsafe { ash::Entry::load()? };

        // Create the instance
        let app_info = vk::ApplicationInfo::builder()
            .application_name(&app_name)
            .application_version(0)
            .engine_name(&app_name)
            .engine_version(0)
            .api_version(vk_version);

        let instance_create_info = vk::InstanceCreateInfo::builder()
            .application_info(&app_info)
            .enabled_layer_names(&layer_names)
            .enabled_extension_names(&instance_extensions);

        let instance = unsafe { entry.create_instance(&instance_create_info, None)? };

        // Create debugging utilities if requested
        let debug = if create_info.debug {
            let debug_info = vk::DebugUtilsMessengerCreateInfoEXT::builder()
                .message_severity(
                    vk::DebugUtilsMessageSeverityFlagsEXT::ERROR
                        | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                        | vk::DebugUtilsMessageSeverityFlagsEXT::INFO,
                )
                .message_type(
                    vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                        | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE
                        | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION,
                )
                .pfn_user_callback(Some(vulkan_debug_callback));
            let debug_utils_loader = ash::extensions::ext::DebugUtils::new(&entry, &instance);
            let debug_messenger =
                unsafe { debug_utils_loader.create_debug_utils_messenger(&debug_info, None)? };
            Some((debug_utils_loader, debug_messenger))
        } else {
            None
        };

        // Create a surface to check for presentation compatibility
        let surface =
            unsafe { ash_window::create_surface(&entry, &instance, create_info.window, None)? };
        let surface_loader = ash::extensions::khr::Surface::new(&entry, &instance);

        // Query for a physical device
        let pd_query = unsafe {
            match pick_physical_device(&instance, surface, &surface_loader, &device_extensions) {
                Some(pd) => pd,
                None => return Err(VulkanBackendCreateError::NoDevice),
            }
        };

        // Cleanup surface since it's not needed anymore
        unsafe {
            surface_loader.destroy_surface(surface, None);
        }

        // Queue requests
        let mut priorities = Vec::with_capacity(pd_query.queue_family_indices.unique.len());
        let mut queue_infos = Vec::with_capacity(pd_query.queue_family_indices.unique.len());
        let mut queue_indices = (0, 0, 0, 0);
        for q in &pd_query.queue_family_indices.unique {
            let mut cur_priorities = Vec::with_capacity(4);

            if pd_query.queue_family_indices.main == *q {
                queue_indices.0 = cur_priorities.len();
                cur_priorities.push(1.0);
            }

            if pd_query.queue_family_indices.transfer == *q {
                queue_indices.1 = cur_priorities.len();
                cur_priorities.push(1.0);
            }

            if pd_query.queue_family_indices.present == *q {
                queue_indices.2 = cur_priorities.len();
                cur_priorities.push(1.0);
            }

            if pd_query.queue_family_indices.compute == *q {
                queue_indices.3 = cur_priorities.len();
                cur_priorities.push(1.0);
            }

            queue_infos.push(
                vk::DeviceQueueCreateInfo::builder()
                    .queue_family_index(*q)
                    .queue_priorities(&cur_priorities)
                    .build(),
            );

            priorities.push(cur_priorities);
        }

        // Request features
        let features = vk::PhysicalDeviceFeatures::builder()
            .fill_mode_non_solid(true)
            .draw_indirect_first_instance(true)
            .multi_draw_indirect(true)
            .depth_clamp(true)
            .build();

        let mut features12 = vk::PhysicalDeviceVulkan12Features::builder()
            .timeline_semaphore(true)
            .buffer_device_address(true)
            .runtime_descriptor_array(true)
            .build();

        let create_info = vk::DeviceCreateInfo::builder()
            .queue_create_infos(&queue_infos)
            .enabled_extension_names(&device_extensions)
            .push_next(&mut features12)
            .enabled_features(&features)
            .build();

        // Create the device
        let device = unsafe { instance.create_device(pd_query.device, &create_info, None)? };

        // Create swapchain loader
        let swapchain_loader = ash::extensions::khr::Swapchain::new(&instance, &device);

        // Create the memory allocator
        let allocator = ManuallyDrop::new(Mutex::new(
            Allocator::new(&AllocatorCreateDesc {
                instance: instance.clone(),
                device: device.clone(),
                physical_device: pd_query.device,
                debug_settings: gpu_allocator::AllocatorDebugSettings {
                    log_memory_information: false,
                    log_leaks_on_shutdown: true,
                    store_stack_traces: false,
                    log_allocations: false,
                    log_frees: false,
                    log_stack_traces: false,
                },
                // TODO: Look into this
                buffer_device_address: false,
            })
            .expect("unable to create GPU memory allocator"),
        ));

        // Create queues
        let main = unsafe {
            VkQueue::new(
                &device,
                device.get_device_queue(pd_query.queue_family_indices.main, queue_indices.0 as u32),
                QueueType::Main,
                pd_query.queue_family_indices.main,
            )?
        };

        let transfer = unsafe {
            VkQueue::new(
                &device,
                device.get_device_queue(
                    pd_query.queue_family_indices.transfer,
                    queue_indices.1 as u32,
                ),
                QueueType::Transfer,
                pd_query.queue_family_indices.transfer,
            )?
        };

        let present = unsafe {
            VkQueue::new(
                &device,
                device.get_device_queue(
                    pd_query.queue_family_indices.present,
                    queue_indices.2 as u32,
                ),
                QueueType::Present,
                pd_query.queue_family_indices.present,
            )?
        };

        let compute = unsafe {
            VkQueue::new(
                &device,
                device.get_device_queue(
                    pd_query.queue_family_indices.compute,
                    queue_indices.3 as u32,
                ),
                QueueType::Compute,
                pd_query.queue_family_indices.compute,
            )?
        };

        let ctx = Self {
            entry,
            instance,
            debug,
            physical_device: pd_query.device,
            queue_family_indices: pd_query.queue_family_indices,
            properties: pd_query.properties,
            features: pd_query.features,
            device,
            surface_loader,
            swapchain_loader,
            main: ShardedLock::new(main),
            transfer: ShardedLock::new(transfer),
            present: ShardedLock::new(present),
            compute: ShardedLock::new(compute),
            allocator,
            render_passes: RenderPassCache::default(),
            framebuffers: FramebufferCache::default(),
            garbage: GarbageCollector::new(),
            resource_state: ShardedLock::new(ResourceState::default()),
        };

        Ok(ctx)
    }
}

impl Drop for VulkanBackend {
    fn drop(&mut self) {
        unsafe {
            self.device.device_wait_idle().unwrap();
            let main = self.main.get_mut().unwrap();
            let transfer = self.transfer.get_mut().unwrap();
            let compute = self.compute.get_mut().unwrap();

            let current = TimelineValues {
                main: main.current_timeline_value(&self.device),
                transfer: transfer.current_timeline_value(&self.device),
                compute: compute.current_timeline_value(&self.device),
            };

            let target = TimelineValues {
                main: main.target_timeline_value(),
                transfer: transfer.target_timeline_value(),
                compute: compute.target_timeline_value(),
            };

            let mut allocator = self.allocator.lock().unwrap();
            self.garbage
                .cleanup(&self.device, &mut allocator, current, target);
            std::mem::drop(allocator);
            std::mem::drop(ManuallyDrop::take(&mut self.allocator));
            self.framebuffers.release(&self.device);
            self.render_passes.release(&self.device);
            self.main.get_mut().unwrap().release(&self.device);
            self.transfer.get_mut().unwrap().release(&self.device);
            self.compute.get_mut().unwrap().release(&self.device);
            self.present.get_mut().unwrap().release(&self.device);
            self.device.destroy_device(None);
            if let Some((loader, messenger)) = &self.debug {
                loader.destroy_debug_utils_messenger(*messenger, None);
            }
            self.instance.destroy_instance(None);
        }
    }
}

impl QueueFamilyIndices {
    // Returns `None` if we can't fill out all queue family types.
    fn find(
        instance: &ash::Instance,
        device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
        surface_loader: &ash::extensions::khr::Surface,
    ) -> Option<QueueFamilyIndices> {
        let mut properties =
            unsafe { instance.get_physical_device_queue_family_properties(device) };
        let mut main = usize::MAX;
        let mut present = usize::MAX;
        let mut transfer = usize::MAX;
        let mut compute = usize::MAX;

        // Find main queue. Probably will end up being family 0.
        for (family_idx, family) in properties.iter().enumerate() {
            if family.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                && family.queue_flags.contains(vk::QueueFlags::TRANSFER)
                && family.queue_flags.contains(vk::QueueFlags::COMPUTE)
            {
                main = family_idx;
                break;
            }
        }

        if main == usize::MAX {
            return None;
        }

        properties[main].queue_count -= 1;

        // Find presentation queue. Would be nice to be different from main.
        for (family_idx, _) in properties.iter().enumerate() {
            let surface_support = unsafe {
                match surface_loader.get_physical_device_surface_support(
                    device,
                    family_idx as u32,
                    surface,
                ) {
                    Ok(support) => support,
                    Err(_) => return None,
                }
            };

            if surface_support && properties[family_idx].queue_count > 0 {
                present = family_idx;
                if family_idx != main {
                    break;
                }
            }
        }

        if present == usize::MAX {
            return None;
        }

        properties[present].queue_count -= 1;

        // Look for a dedicated transfer queue. Supported on some devices. Fallback is main.
        for (family_idx, family) in properties.iter().enumerate() {
            if family.queue_flags.contains(vk::QueueFlags::TRANSFER)
                && properties[family_idx].queue_count > 0
            {
                transfer = family_idx;
                if family_idx != main && family_idx != present {
                    break;
                }
            }
        }

        if transfer == usize::MAX {
            return None;
        }

        properties[transfer].queue_count -= 1;

        // Look for a dedicated async compute queue. Supported on some devices. Fallback is main.
        for (family_idx, family) in properties.iter().enumerate() {
            if family.queue_flags.contains(vk::QueueFlags::COMPUTE)
                && properties[family_idx].queue_count > 0
            {
                compute = family_idx;
                if family_idx != main && family_idx != present && family_idx != transfer {
                    break;
                }
            }
        }

        if compute == usize::MAX {
            return None;
        }

        let unique = {
            let mut qfi_set = std::collections::HashSet::<usize>::new();
            qfi_set.insert(main);
            qfi_set.insert(present);
            qfi_set.insert(transfer);
            qfi_set.insert(compute);

            let mut unique_qfis = Vec::with_capacity(qfi_set.len());
            for q in qfi_set {
                unique_qfis.push(q as u32);
            }

            unique_qfis
        };

        Some(QueueFamilyIndices {
            main: main as u32,
            present: present as u32,
            transfer: transfer as u32,
            compute: compute as u32,
            unique,
        })
    }
}

unsafe fn pick_physical_device(
    instance: &ash::Instance,
    surface: vk::SurfaceKHR,
    loader: &ash::extensions::khr::Surface,
    extensions: &[*const i8],
) -> Option<PhysicalDeviceQuery> {
    let devices = match instance.enumerate_physical_devices() {
        Ok(devices) => devices,
        Err(_) => return None,
    };

    let mut device_type = vk::PhysicalDeviceType::OTHER;
    let mut query = None;
    for device in devices {
        let properties = instance.get_physical_device_properties(device);
        let features = instance.get_physical_device_features(device);

        // Must support requested extensions
        if check_device_extensions(instance, device, extensions).is_some() {
            continue;
        }

        // Must support surface stuff
        let formats = match loader.get_physical_device_surface_formats(device, surface) {
            Ok(formats) => formats,
            Err(_) => continue,
        };

        let present_modes = match loader.get_physical_device_surface_present_modes(device, surface)
        {
            Ok(modes) => modes,
            Err(_) => continue,
        };

        if formats.is_empty() || present_modes.is_empty() {
            continue;
        }

        // Must support all queue family indices
        let qfi = QueueFamilyIndices::find(instance, device, surface, loader);
        if qfi.is_none() {
            continue;
        }

        // Pick this device if it's better than the old one
        if device_type_rank(properties.device_type) >= device_type_rank(device_type) {
            device_type = properties.device_type;
            query = Some(PhysicalDeviceQuery {
                device,
                features,
                properties,
                queue_family_indices: qfi.unwrap(),
            });
        }
    }

    query
}

/// Check that a physical devices supports required device extensions.
///
/// Returns `None` on a success, or `Some` containing the name of the missing extension.
unsafe fn check_device_extensions(
    instance: &ash::Instance,
    device: vk::PhysicalDevice,
    extensions: &[*const i8],
) -> Option<String> {
    let found_extensions = match instance.enumerate_device_extension_properties(device) {
        Ok(extensions) => extensions,
        Err(_) => return Some(String::default()),
    };

    for extension_name in extensions {
        let mut found = false;
        for extension_property in &found_extensions {
            let s = CStr::from_ptr(extension_property.extension_name.as_ptr());

            if CStr::from_ptr(*extension_name).eq(s) {
                found = true;
                break;
            }
        }

        if !found {
            return Some(String::from(
                CStr::from_ptr(*extension_name).to_str().unwrap(),
            ));
        }
    }

    None
}

fn device_type_rank(ty: vk::PhysicalDeviceType) -> u32 {
    match ty {
        vk::PhysicalDeviceType::DISCRETE_GPU => 4,
        vk::PhysicalDeviceType::INTEGRATED_GPU => 3,
        vk::PhysicalDeviceType::CPU => 2,
        vk::PhysicalDeviceType::VIRTUAL_GPU => 1,
        _ => 0,
    }
}

/// Given a slice of commands, loops until we find an `EndRenderPass` command and records all
/// render pass dependent resources.
unsafe fn find_render_pass_resources(
    descriptor: &RenderPassDescriptor<'_, crate::VulkanBackend>,
    commands: &[Command<'_, crate::VulkanBackend>],
    device: &ash::Device,
    command_buffer: vk::CommandBuffer,
    tracker: &mut PipelineTracker,
) {
    let mut buffers = Vec::default();
    let mut images = Vec::with_capacity(descriptor.color_attachments.len());

    // Handle images
    for attachment in &descriptor.color_attachments {
        let image = match attachment.source {
            ColorAttachmentSource::SurfaceImage(image) => image.internal().image(),
        };
        images.push((
            image,
            ResourceUsage {
                used: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                access: vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::COLOR_ATTACHMENT_READ,
            },
        ));
    }

    // Handle vertex/index buffers
    for command in commands {
        match command {
            Command::BindVertexBuffers { binds, .. } => {
                for bind in binds {
                    buffers.push((
                        bind.buffer.internal().buffer,
                        ResourceUsage {
                            used: vk::PipelineStageFlags::VERTEX_INPUT,
                            access: vk::AccessFlags::VERTEX_ATTRIBUTE_READ,
                        },
                    ));
                }
            }
            Command::BindIndexBuffer { buffer, .. } => buffers.push((
                buffer.internal().buffer,
                ResourceUsage {
                    used: vk::PipelineStageFlags::VERTEX_INPUT,
                    access: vk::AccessFlags::INDEX_READ,
                },
            )),
            Command::EndRenderPass => break,
            _ => {}
        }
    }

    if let Some(barrier) = tracker.update(&buffers, &images) {
        barrier.execute(device, command_buffer);
    }
}

unsafe extern "system" fn vulkan_debug_callback(
    message_severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    _user_data: *mut std::os::raw::c_void,
) -> vk::Bool32 {
    let callback_data = *p_callback_data;
    let message_id_number: i32 = callback_data.message_id_number as i32;

    let message_id_name = if callback_data.p_message_id_name.is_null() {
        Cow::from("")
    } else {
        CStr::from_ptr(callback_data.p_message_id_name).to_string_lossy()
    };

    let message = if callback_data.p_message.is_null() {
        Cow::from("")
    } else {
        CStr::from_ptr(callback_data.p_message).to_string_lossy()
    };

    match message_severity {
        DebugUtilsMessageSeverityFlagsEXT::VERBOSE => print!(
            "{:?}:\n{:?} [{} ({})] : {}\n",
            message_severity,
            message_type,
            message_id_name,
            &message_id_number.to_string(),
            message,
        ),
        DebugUtilsMessageSeverityFlagsEXT::INFO => print!(
            "{:?}:\n{:?} [{} ({})] : {}\n",
            message_severity,
            message_type,
            message_id_name,
            &message_id_number.to_string(),
            message,
        ),
        DebugUtilsMessageSeverityFlagsEXT::WARNING => print!(
            "{:?}:\n{:?} [{} ({})] : {}\n",
            message_severity,
            message_type,
            message_id_name,
            &message_id_number.to_string(),
            message,
        ),
        DebugUtilsMessageSeverityFlagsEXT::ERROR => print!(
            "{:?}:\n{:?} [{} ({})] : {}\n",
            message_severity,
            message_type,
            message_id_name,
            &message_id_number.to_string(),
            message,
        ),
        _ => {}
    }

    vk::FALSE
}

impl From<vk::Result> for VulkanBackendCreateError {
    fn from(res: vk::Result) -> Self {
        VulkanBackendCreateError::Vulkan(res)
    }
}

impl From<ash::LoadingError> for VulkanBackendCreateError {
    fn from(err: ash::LoadingError) -> Self {
        VulkanBackendCreateError::AshLoadError(err)
    }
}