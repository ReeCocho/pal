use crate::{
    buffer::Buffer,
    command_buffer::Command,
    descriptor_set::DescriptorSet,
    graphics_pipeline::GraphicsPipeline,
    surface::SurfaceImage,
    texture::Texture,
    types::{IndexType, LoadOp, ShaderStage, StoreOp},
    Backend,
};

pub struct RenderPassDescriptor<'a, B: Backend> {
    pub color_attachments: Vec<ColorAttachment<'a, B>>,
    pub depth_stencil_attachment: Option<DepthStencilAttachment<'a, B>>,
}

pub struct ColorAttachment<'a, B: Backend> {
    pub source: ColorAttachmentSource<'a, B>,
    pub load_op: LoadOp,
    pub store_op: StoreOp,
}

pub enum ColorAttachmentSource<'a, B: Backend> {
    SurfaceImage(&'a SurfaceImage<B>),
    Texture {
        texture: &'a Texture<B>,
        array_element: usize,
        mip_level: usize,
    },
}

pub struct DepthStencilAttachment<'a, B: Backend> {
    pub texture: &'a Texture<B>,
    pub array_element: usize,
    pub mip_level: usize,
    pub load_op: LoadOp,
    pub store_op: StoreOp,
}

pub struct RenderPass<'a, B: Backend> {
    pub(crate) commands: Vec<Command<'a, B>>,
}

pub struct VertexBind<'a, B: Backend> {
    pub buffer: &'a Buffer<B>,
    pub array_element: usize,
    pub offset: u64,
}

impl<'a, B: Backend> RenderPass<'a, B> {
    #[inline]
    pub fn bind_pipeline(&mut self, pipeline: GraphicsPipeline<B>) {
        self.commands.push(Command::BindGraphicsPipeline(pipeline));
    }

    #[inline]
    pub fn bind_sets(&mut self, first: usize, sets: Vec<&'a DescriptorSet<B>>) {
        self.commands.push(Command::BindDescriptorSets {
            sets,
            first,
            stage: ShaderStage::AllGraphics,
        });
    }

    #[inline]
    pub fn bind_vertex_buffers(&mut self, first: usize, binds: Vec<VertexBind<'a, B>>) {
        self.commands
            .push(Command::BindVertexBuffers { first, binds });
    }

    #[inline]
    pub fn bind_index_buffer(
        &mut self,
        buffer: &'a Buffer<B>,
        array_element: usize,
        offset: u64,
        ty: IndexType,
    ) {
        self.commands.push(Command::BindIndexBuffer {
            buffer,
            array_element,
            offset,
            ty,
        });
    }

    #[inline]
    pub fn draw(
        &mut self,
        vertex_count: usize,
        instance_count: usize,
        first_vertex: usize,
        first_instance: usize,
    ) {
        self.commands.push(Command::Draw {
            vertex_count,
            instance_count,
            first_vertex,
            first_instance,
        });
    }

    #[inline]
    pub fn draw_indexed(
        &mut self,
        index_count: usize,
        instance_count: usize,
        first_index: usize,
        vertex_offset: isize,
        first_instance: usize,
    ) {
        self.commands.push(Command::DrawIndexed {
            index_count,
            instance_count,
            first_index,
            vertex_offset,
            first_instance,
        });
    }

    #[inline]
    pub fn draw_indexed_indirect(
        &mut self,
        buffer: &'a Buffer<B>,
        array_element: usize,
        offset: u64,
        draw_count: usize,
        stride: u64,
    ) {
        self.commands.push(Command::DrawIndexedIndirect {
            buffer,
            array_element,
            offset,
            draw_count,
            stride,
        });
    }
}
