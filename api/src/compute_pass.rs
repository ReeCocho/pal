use crate::{
    command_buffer::Command, compute_pipeline::ComputePipeline, descriptor_set::DescriptorSet,
    types::ShaderStage, Backend,
};

pub struct ComputePass<'a, B: Backend> {
    pub(crate) commands: Vec<Command<'a, B>>,
}

impl<'a, B: Backend> ComputePass<'a, B> {
    #[inline]
    pub fn bind_pipeline(&mut self, pipeline: ComputePipeline<B>) {
        self.commands.push(Command::BindComputePipeline(pipeline));
    }

    #[inline]
    pub fn bind_sets(&mut self, first: usize, sets: Vec<&'a DescriptorSet<B>>) {
        self.commands.push(Command::BindDescriptorSets {
            sets,
            first,
            stage: ShaderStage::Compute,
        });
    }

    #[inline]
    pub fn dispatch(&mut self, x: u32, y: u32, z: u32) {
        self.commands.push(Command::Dispatch(x, y, z));
    }
}
