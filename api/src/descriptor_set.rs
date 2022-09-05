use std::sync::Arc;
use thiserror::Error;

use crate::{
    buffer::Buffer,
    context::Context,
    types::{AccessType, ShaderStage},
    Backend,
};

pub struct DescriptorSetCreateInfo<B: Backend> {
    pub layout: DescriptorSetLayout<B>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DescriptorSetLayoutCreateInfo {
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
    StorageBuffer(AccessType),
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

pub struct DescriptorSetUpdate<'a, B: Backend> {
    pub binding: u32,
    pub array_element: usize,
    pub value: DescriptorValue<'a, B>,
}

pub enum DescriptorValue<'a, B: Backend> {
    UniformBuffer {
        buffer: &'a Buffer<B>,
        array_element: usize,
    },
    StorageBuffer {
        buffer: &'a Buffer<B>,
        array_element: usize,
    },
    Texture,
}

pub(crate) struct DescriptorSetLayoutInner<B: Backend> {
    ctx: Context<B>,
    pub(crate) id: B::DescriptorSetLayout,
}

impl<B: Backend> DescriptorSet<B> {
    #[inline(always)]
    pub fn new(
        ctx: Context<B>,
        create_info: DescriptorSetCreateInfo<B>,
    ) -> Result<Self, DescriptorSetCreateError> {
        let layout = create_info.layout.clone();
        let id = unsafe { ctx.0.create_descriptor_set(create_info)? };
        Ok(Self { ctx, layout, id })
    }

    #[inline(always)]
    pub fn internal(&self) -> &B::DescriptorSet {
        &self.id
    }

    #[inline(always)]
    pub fn layout(&self) -> &DescriptorSetLayout<B> {
        &self.layout
    }

    pub fn update(&mut self, updates: &[DescriptorSetUpdate<B>]) {
        unsafe {
            self.ctx
                .0
                .update_descriptor_sets(&mut self.id, &self.layout.0.id, updates);
        }
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
        ctx: Context<B>,
        create_info: DescriptorSetLayoutCreateInfo,
    ) -> Result<Self, DescriptorSetLayoutCreateError> {
        let id = unsafe { ctx.0.create_descriptor_set_layout(create_info)? };
        Ok(Self(Arc::new(DescriptorSetLayoutInner { ctx, id })))
    }

    #[inline(always)]
    pub fn internal(&self) -> &B::DescriptorSetLayout {
        &self.0.id
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
