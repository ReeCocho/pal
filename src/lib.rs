#[cfg(feature = "vulkan")]
type Backend = vulkan::VulkanBackend;

pub mod prelude {
    pub use api::types::*;

    // Context
    pub type Context = api::context::Context<crate::Backend>;

    // Surface
    pub type Surface = api::surface::Surface<crate::Backend>;
    pub type SurfaceImage = api::surface::SurfaceImage<crate::Backend>;
    pub use api::surface::{
        SurfaceConfiguration, SurfaceCreateError, SurfaceCreateInfo, SurfacePresentSuccess,
    };

    // Render pass
    pub use api::render_pass::{
        ColorAttachment, ColorAttachmentSource, DepthStencilAttachment, RenderPass,
        RenderPassDescriptor, VertexBind,
    };

    // Command buffer
    pub use api::command_buffer::CopyBufferToBuffer;

    // Queue
    pub type Queue = api::queue::Queue<crate::Backend>;

    // Shader
    pub type Shader = api::shader::Shader<crate::Backend>;
    pub use api::shader::{ShaderCreateError, ShaderCreateInfo};

    // Graphics pipeline
    pub type GraphicsPipeline = api::graphics_pipeline::GraphicsPipeline<crate::Backend>;
    pub use api::graphics_pipeline::{
        ColorBlendAttachment, ColorBlendState, DepthStencilState, GraphicsPipelineCreateError,
        GraphicsPipelineCreateInfo, RasterizationState, ShaderStages, VertexInputAttribute,
        VertexInputBinding, VertexInputState,
    };

    // Compute pipeline
    pub type ComputePipeline = api::compute_pipeline::ComputePipeline<crate::Backend>;
    pub use api::compute_pipeline::{ComputePipelineCreateError, ComputePipelineCreateInfo};

    // Buffer
    pub type Buffer = api::buffer::Buffer<crate::Backend>;
    pub use api::buffer::{
        BufferCreateError, BufferCreateInfo, BufferReadView, BufferViewError, BufferWriteView,
    };

    // Texture
    pub type Texture = api::texture::Texture<crate::Backend>;
    pub use api::texture::{Sampler, TextureCreateError, TextureCreateInfo};

    // Descriptor set & layout
    pub type DescriptorSetLayout = api::descriptor_set::DescriptorSetLayout<crate::Backend>;
    pub type DescriptorSet = api::descriptor_set::DescriptorSet<crate::Backend>;
    pub use api::descriptor_set::{
        DescriptorBinding, DescriptorSetCreateError, DescriptorSetCreateInfo,
        DescriptorSetLayoutCreateError, DescriptorSetLayoutCreateInfo, DescriptorSetUpdate,
        DescriptorType, DescriptorValue,
    };
}
