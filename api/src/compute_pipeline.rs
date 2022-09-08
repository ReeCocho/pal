use crate::{context::Context, descriptor_set::DescriptorSetLayout, shader::Shader, Backend};
use std::sync::Arc;
use thiserror::Error;

pub struct ComputePipelineCreateInfo<B: Backend> {
    pub layouts: Vec<DescriptorSetLayout<B>>,
    pub module: Shader<B>,
    pub work_group_size: (u32, u32, u32),
    pub debug_name: Option<String>,
}

#[derive(Debug, Error)]
pub enum ComputePipelineCreateError {
    #[error("an error occured: {0}")]
    Other(String),
}

pub struct ComputePipeline<B: Backend>(Arc<ComputePipelineInner<B>>);

pub(crate) struct ComputePipelineInner<B: Backend> {
    ctx: Context<B>,
    pub(crate) layouts: Vec<DescriptorSetLayout<B>>,
    pub(crate) id: B::ComputePipeline,
}

impl<B: Backend> ComputePipeline<B> {
    pub fn new(
        ctx: Context<B>,
        create_info: ComputePipelineCreateInfo<B>,
    ) -> Result<Self, ComputePipelineCreateError> {
        let layouts = create_info.layouts.clone();
        let id = unsafe { ctx.0.create_compute_pipeline(create_info)? };
        Ok(Self(Arc::new(ComputePipelineInner { ctx, id, layouts })))
    }

    #[inline(always)]
    pub fn internal(&self) -> &B::ComputePipeline {
        &self.0.id
    }

    #[inline(always)]
    pub fn layouts(&self) -> &[DescriptorSetLayout<B>] {
        &self.0.layouts
    }
}

impl<B: Backend> Drop for ComputePipelineInner<B> {
    fn drop(&mut self) {
        unsafe {
            self.ctx.0.destroy_compute_pipeline(&mut self.id);
        }
    }
}

impl<B: Backend> Clone for ComputePipeline<B> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
