use crate::{
    buffer::Buffer,
    compute_pass::ComputePass,
    compute_pipeline::ComputePipeline,
    descriptor_set::DescriptorSet,
    graphics_pipeline::GraphicsPipeline,
    render_pass::{RenderPass, RenderPassDescriptor, VertexBind},
    texture::Texture,
    types::{IndexType, QueueType, ShaderStage},
    Backend,
};

pub struct CopyBufferToBuffer<'a, B: Backend> {
    /// The source buffer to read from.
    pub src: &'a Buffer<B>,
    /// The source array element to read from.
    pub src_array_element: usize,
    /// The offset within the array element of the source buffer to read from.
    pub src_offset: u64,
    /// The destination buffer to write to.
    pub dst: &'a Buffer<B>,
    /// The destination array element to write to.
    pub dst_array_element: usize,
    /// The offset within the array element of the destination buffer to write to.
    pub dst_offset: u64,
    /// The number of bytes to copy.
    pub len: u64,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BufferTextureCopy {
    pub buffer_offset: u64,
    pub buffer_row_length: u32,
    pub buffer_image_height: u32,
    /// The array element of the buffer to read/write.
    pub buffer_array_element: usize,
    /// The width, height, and depth offsets within the texture to read/write.
    pub texture_offset: (u32, u32, u32),
    /// The width, height, and depth sizes within the texture to read/write.
    pub texture_extent: (u32, u32, u32),
    /// The mip level of the texture to read/write.
    pub texture_mip_level: usize,
    /// The array element of the texture to read/write.
    pub texture_array_element: usize,
}

pub enum Command<'a, B: Backend> {
    BeginRenderPass(RenderPassDescriptor<'a, B>),
    EndRenderPass,
    BeginComputePass,
    EndComputePass,
    BindGraphicsPipeline(GraphicsPipeline<B>),
    BindComputePipeline(ComputePipeline<B>),
    Dispatch(u32, u32, u32),
    BindDescriptorSets {
        sets: Vec<&'a DescriptorSet<B>>,
        first: usize,
        stage: ShaderStage,
    },
    BindVertexBuffers {
        first: usize,
        binds: Vec<VertexBind<'a, B>>,
    },
    BindIndexBuffer {
        buffer: &'a Buffer<B>,
        array_element: usize,
        offset: u64,
        ty: IndexType,
    },
    Draw {
        vertex_count: usize,
        instance_count: usize,
        first_vertex: usize,
        first_instance: usize,
    },
    DrawIndexed {
        index_count: usize,
        instance_count: usize,
        first_index: usize,
        vertex_offset: isize,
        first_instance: usize,
    },
    DrawIndexedIndirect {
        buffer: &'a Buffer<B>,
        array_element: usize,
        offset: u64,
        draw_count: usize,
        stride: u64,
    },
    CopyBufferToBuffer(CopyBufferToBuffer<'a, B>),
    CopyBufferToTexture {
        buffer: &'a Buffer<B>,
        texture: &'a Texture<B>,
        copy: BufferTextureCopy,
    },
    CopyTextureToBuffer {
        buffer: &'a Buffer<B>,
        texture: &'a Texture<B>,
        copy: BufferTextureCopy,
    },
}

/// A command buffer is used to record commands which are the submitted to a queue.
pub struct CommandBuffer<'a, B: Backend> {
    pub(crate) queue_ty: QueueType,
    pub(crate) commands: Vec<Command<'a, B>>,
}

impl<'a, B: Backend> CommandBuffer<'a, B> {
    /// Begins a render pass scope.
    ///
    /// # Arguments
    /// - `descriptor` - A description of the render pass.
    /// - `pass` - A function that records render pass commands.
    ///
    /// # Panics
    /// - If the queue type this command buffer was created with does not support graphics
    /// commands.
    ///
    pub fn render_pass(
        &mut self,
        descriptor: RenderPassDescriptor<'a, B>,
        pass: impl FnOnce(&mut RenderPass<'a, B>),
    ) {
        assert_eq!(
            self.queue_ty,
            QueueType::Main,
            "queue `{:?}` does not support render passes",
            self.queue_ty
        );

        self.commands.push(Command::BeginRenderPass(descriptor));
        let mut render_pass = RenderPass {
            commands: Vec::default(),
        };
        pass(&mut render_pass);
        self.commands.extend(render_pass.commands);
        self.commands.push(Command::EndRenderPass);
    }

    /// Begins a compute pass scope.
    ///
    /// # Arguments
    /// - `pass` - A function that records compute commands.
    ///
    /// # Panics
    /// - If the queue type this command buffer was created with does not support compute commands.
    pub fn compute_pass(&mut self, pass: impl FnOnce(&mut ComputePass<'a, B>)) {
        assert!(
            self.queue_ty == QueueType::Main || self.queue_ty == QueueType::Compute,
            "queue `{:?}` does not support compute passes",
            self.queue_ty
        );

        self.commands.push(Command::BeginComputePass);
        let mut compute_pass = ComputePass {
            commands: Vec::default(),
        };
        pass(&mut compute_pass);
        self.commands.extend(compute_pass.commands);
        self.commands.push(Command::EndComputePass);
    }

    /// Copies data from one buffer into another.
    ///
    /// # Arguments
    /// - `copy` - A description of the copy to perform.
    ///
    /// # Panics
    /// - If the queue type this command buffer was created with does not support transfer
    /// commands.
    #[inline(always)]
    pub fn copy_buffer_to_buffer(&mut self, copy: CopyBufferToBuffer<'a, B>) {
        assert!(
            self.queue_ty == QueueType::Main || self.queue_ty == QueueType::Transfer,
            "queue `{:?}` does not support transfer commands",
            self.queue_ty
        );

        self.commands.push(Command::CopyBufferToBuffer(copy));
    }

    /// Copies data from a buffer into a texture.
    ///
    /// # Arguments
    /// - `texture` - The destination texture to write to.
    /// - `buffer` - The source buffer to copy from.
    /// - `copy` - A description of the copy to perform.
    ///
    /// # Panics
    /// - If the queue type this command buffer was created with does not support transfer
    /// commands.
    #[inline(always)]
    pub fn copy_buffer_to_texture(
        &mut self,
        texture: &'a Texture<B>,
        buffer: &'a Buffer<B>,
        copy: BufferTextureCopy,
    ) {
        assert!(
            self.queue_ty == QueueType::Main || self.queue_ty == QueueType::Transfer,
            "queue `{:?}` does not support transfer commands",
            self.queue_ty
        );

        self.commands.push(Command::CopyBufferToTexture {
            buffer,
            texture,
            copy,
        });
    }

    /// Copies data from a texture into a buffer.
    ///
    /// # Arguments
    /// - `buffer` - The destination buffer to write to.
    /// - `texture` The source texture to read from.
    /// - `copy` - A description of the copy to perform.
    ///
    /// # Panics
    /// - If the queue type this command buffer was created with does not support transfer
    /// commands.
    #[inline(always)]
    pub fn copy_texture_to_buffer(
        &mut self,
        buffer: &'a Buffer<B>,
        texture: &'a Texture<B>,
        copy: BufferTextureCopy,
    ) {
        assert!(
            self.queue_ty == QueueType::Main || self.queue_ty == QueueType::Transfer,
            "queue `{:?}` does not support transfer commands",
            self.queue_ty
        );

        self.commands.push(Command::CopyTextureToBuffer {
            buffer,
            texture,
            copy,
        });
    }
}
