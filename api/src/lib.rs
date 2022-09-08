pub mod buffer;
pub mod command_buffer;
pub mod compute_pass;
pub mod compute_pipeline;
pub mod context;
pub mod descriptor_set;
pub mod graphics_pipeline;
pub mod queue;
pub mod render_pass;
pub mod shader;
pub mod surface;
pub mod texture;
pub mod types;

use std::{ptr::NonNull, time::Duration};

use buffer::{BufferCreateError, BufferCreateInfo, BufferViewError};
use command_buffer::Command;
use compute_pipeline::{ComputePipelineCreateError, ComputePipelineCreateInfo};
use descriptor_set::{
    DescriptorSetCreateError, DescriptorSetCreateInfo, DescriptorSetLayoutCreateError,
    DescriptorSetLayoutCreateInfo, DescriptorSetUpdate,
};
use graphics_pipeline::{GraphicsPipelineCreateError, GraphicsPipelineCreateInfo};
use queue::SurfacePresentFailure;
use raw_window_handle::HasRawWindowHandle;
use shader::{ShaderCreateError, ShaderCreateInfo};
use surface::{
    SurfaceConfiguration, SurfaceCreateError, SurfaceCreateInfo, SurfaceImageAcquireError,
    SurfacePresentSuccess, SurfaceUpdateError,
};
use texture::TextureCreateInfo;
use types::{JobStatus, QueueType};

pub trait Backend: Sized + 'static {
    type Buffer;
    type Texture;
    type Surface;
    type SurfaceImage;
    type Shader;
    type GraphicsPipeline;
    type ComputePipeline;
    type DescriptorSetLayout;
    type DescriptorSet;
    type Job;

    unsafe fn create_surface<'a, W: HasRawWindowHandle>(
        &self,
        create_info: SurfaceCreateInfo<'a, W>,
    ) -> Result<Self::Surface, SurfaceCreateError>;
    unsafe fn destroy_surface(&self, id: &mut Self::Surface);
    unsafe fn update_surface(
        &self,
        id: &mut Self::Surface,
        config: SurfaceConfiguration,
    ) -> Result<(), SurfaceUpdateError>;
    unsafe fn acquire_image(
        &self,
        id: &mut Self::Surface,
    ) -> Result<Self::SurfaceImage, SurfaceImageAcquireError>;
    unsafe fn destroy_surface_image(&self, id: &mut Self::SurfaceImage);

    unsafe fn submit_commands<'a>(
        &self,
        queue: QueueType,
        debug_name: Option<&str>,
        commands: Vec<Command<'a, Self>>,
    ) -> Self::Job;
    unsafe fn present_image(
        &self,
        surface: &Self::Surface,
        image: &mut Self::SurfaceImage,
    ) -> Result<SurfacePresentSuccess, SurfacePresentFailure>;
    unsafe fn wait_on(&self, job: &Self::Job, timeout: Option<Duration>) -> JobStatus;
    unsafe fn poll_status(&self, job: &Self::Job) -> JobStatus;

    unsafe fn create_buffer(
        &self,
        create_info: BufferCreateInfo,
    ) -> Result<Self::Buffer, BufferCreateError>;
    unsafe fn create_texture(&self, create_info: TextureCreateInfo<Self>) -> Self::Texture;
    unsafe fn create_shader(
        &self,
        create_info: ShaderCreateInfo,
    ) -> Result<Self::Shader, ShaderCreateError>;
    unsafe fn create_graphics_pipeline(
        &self,
        create_info: GraphicsPipelineCreateInfo<Self>,
    ) -> Result<Self::GraphicsPipeline, GraphicsPipelineCreateError>;
    unsafe fn create_compute_pipeline(
        &self,
        create_info: ComputePipelineCreateInfo<Self>,
    ) -> Result<Self::ComputePipeline, ComputePipelineCreateError>;
    unsafe fn create_descriptor_set(
        &self,
        create_info: DescriptorSetCreateInfo<Self>,
    ) -> Result<Self::DescriptorSet, DescriptorSetCreateError>;
    unsafe fn create_descriptor_set_layout(
        &self,
        create_info: DescriptorSetLayoutCreateInfo,
    ) -> Result<Self::DescriptorSetLayout, DescriptorSetLayoutCreateError>;
    unsafe fn destroy_buffer(&self, id: &mut Self::Buffer);
    unsafe fn destroy_texture(&self, id: &mut Self::Texture);
    unsafe fn destroy_shader(&self, id: &mut Self::Shader);
    unsafe fn destroy_graphics_pipeline(&self, id: &mut Self::GraphicsPipeline);
    unsafe fn destroy_compute_pipeline(&self, id: &mut Self::ComputePipeline);
    unsafe fn destroy_descriptor_set(&self, id: &mut Self::DescriptorSet);
    unsafe fn destroy_descriptor_set_layout(&self, id: &mut Self::DescriptorSetLayout);

    unsafe fn map_memory(
        &self,
        id: &mut Self::Buffer,
        idx: usize,
    ) -> Result<(NonNull<u8>, u64), BufferViewError>;
    unsafe fn unmap_memory(&self, id: &mut Self::Buffer);
    unsafe fn flush_range(&self, id: &mut Self::Buffer, idx: usize);
    unsafe fn invalidate_range(&self, id: &mut Self::Buffer, idx: usize);

    unsafe fn update_descriptor_sets(
        &self,
        id: &mut Self::DescriptorSet,
        layout: &Self::DescriptorSetLayout,
        updates: &[DescriptorSetUpdate<Self>],
    );
}
