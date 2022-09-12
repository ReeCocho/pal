use crate::{
    buffer::Buffer,
    compute_pass::ComputePass,
    compute_pipeline::ComputePipeline,
    descriptor_set::DescriptorSet,
    graphics_pipeline::GraphicsPipeline,
    render_pass::{RenderPass, RenderPassDescriptor, VertexBind},
    types::{IndexType, ShaderStage},
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

    pub fn compute_pass(&mut self, pass: impl FnOnce(&mut ComputePass<'a, B>)) {
        self.commands.push(Command::BeginComputePass);
        let mut compute_pass = ComputePass {
            commands: Vec::default(),
        };
        pass(&mut compute_pass);
        self.commands.extend(compute_pass.commands);
        self.commands.push(Command::EndComputePass);
    }

    #[inline(always)]
    pub fn copy_buffer_to_buffer(&mut self, copy: CopyBufferToBuffer<'a, B>) {
        self.commands.push(Command::CopyBufferToBuffer(copy));
    }
}
