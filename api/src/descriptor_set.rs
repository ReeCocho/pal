use std::sync::Arc;
use thiserror::Error;

use crate::{context::Context, types::ShaderStage, Backend};

pub struct DescriptorSetCreateInfo<B: Backend> {
    pub ctx: Context<B>,
    pub layout: DescriptorSetLayout<B>,
}

pub struct DescriptorSetLayoutCreateInfo<B: Backend> {
    pub ctx: Context<B>,
    pub bindings: Vec<DescriptorBinding>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DescriptorBinding {
    pub binding: u32,
    pub ty: DescriptorType,
    pub count: usize,
    pub stage: ShaderStage,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DescriptorType {
    Texture,
    UniformBuffer,
    StorageBuffer,
}

#[derive(Debug, Error)]
pub enum DescriptorSetLayoutCreateError {
    #[error("an error has occured: {0}")]
    Other(String),
}

#[derive(Debug, Error)]
pub enum DescriptorSetCreateError {
    #[error("an error has occured: {0}")]
    Other(String),
}

pub struct DescriptorSetLayout<B: Backend>(Arc<DescriptorSetLayoutInner<B>>);

pub struct DescriptorSet<B: Backend> {
    ctx: Context<B>,
    layout: DescriptorSetLayout<B>,
    pub(crate) id: B::DescriptorSet,
}

pub(crate) struct DescriptorSetLayoutInner<B: Backend> {
    ctx: Context<B>,
    pub(crate) id: B::DescriptorSetLayout,
}

impl<B: Backend> DescriptorSet<B> {
    #[inline(always)]
    pub fn new(create_info: DescriptorSetCreateInfo<B>) -> Result<Self, DescriptorSetCreateError> {
        let ctx = create_info.ctx.clone();
        let layout = create_info.layout.clone();
        let id = unsafe { ctx.0.create_descriptor_set(create_info)? };
        Ok(Self { ctx, layout, id })
    }

    #[inline(always)]
    pub fn layout(&self) -> &DescriptorSetLayout<B> {
        &self.layout
    }
}

impl<B: Backend> Drop for DescriptorSet<B> {
    #[inline(always)]
    fn drop(&mut self) {
        unsafe {
            self.ctx.0.destroy_descriptor_set(&mut self.id);
        }
    }
}

impl<B: Backend> DescriptorSetLayout<B> {
    #[inline(always)]
    pub fn new(
        create_info: DescriptorSetLayoutCreateInfo<B>,
    ) -> Result<Self, DescriptorSetLayoutCreateError> {
        let ctx = create_info.ctx.clone();
        let id = unsafe { ctx.0.create_descriptor_set_layout(create_info)? };
        Ok(Self(Arc::new(DescriptorSetLayoutInner { ctx, id })))
    }
}

impl<B: Backend> Clone for DescriptorSetLayout<B> {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<B: Backend> Drop for DescriptorSetLayoutInner<B> {
    #[inline(always)]
    fn drop(&mut self) {
        unsafe {
            self.ctx.0.destroy_descriptor_set_layout(&mut self.id);
        }
    }
}
