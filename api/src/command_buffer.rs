use crate::{
    buffer::Buffer,
    graphics_pipeline::GraphicsPipeline,
    render_pass::{RenderPass, RenderPassDescriptor, VertexBind},
    types::IndexType,
    Backend,
};

pub struct CopyBufferToBuffer<'a, B: Backend> {
    pub src: &'a Buffer<B>,
    pub src_array_element: usize,
    pub src_offset: u64,
    pub dst: &'a Buffer<B>,
    pub dst_array_element: usize,
    pub dst_offset: u64,
    pub len: u64,
}

pub enum Command<'a, B: Backend> {
    BeginRenderPass(RenderPassDescriptor<'a, B>),
    EndRenderPass,
    BindGraphicsPipeline(GraphicsPipeline<B>),
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
    DrawIndexed {
        index_count: usize,
        instance_count: usize,
        first_index: usize,
        vertex_offset: isize,
        first_instance: usize,
    },
    CopyBufferToBuffer(CopyBufferToBuffer<'a, B>),
}

pub struct CommandBuffer<'a, B: Backend> {
    pub(crate) commands: Vec<Command<'a, B>>,
}

impl<'a, B: Backend> CommandBuffer<'a, B> {
    pub fn render_pass(
        &mut self,
        descriptor: RenderPassDescriptor<'a, B>,
        pass: impl FnOnce(&mut RenderPass<'a, B>),
    ) {
        self.commands.push(Command::BeginRenderPass(descriptor));
        let mut render_pass = RenderPass {
            commands: Vec::default(),
        };
        pass(&mut render_pass);
        self.commands.extend(render_pass.commands);
        self.commands.push(Command::EndRenderPass);
    }

    #[inline(always)]
    pub fn copy_buffer_to_buffer(&mut self, copy: CopyBufferToBuffer<'a, B>) {
        self.commands.push(Command::CopyBufferToBuffer(copy));
    }
}
