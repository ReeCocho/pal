use crate::{
    context::Context,
    types::{ImageType, MemoryUsage, TextureFormat, TextureUsage},
    Backend,
};
use thiserror::Error;

pub struct TextureCreateInfo {
    pub format: TextureFormat,
    pub ty: ImageType,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub array_elements: usize,
    pub mip_levels: usize,
    pub image_usage: TextureUsage,
    pub memory_usage: MemoryUsage,
    pub debug_name: Option<String>,
}

#[derive(Debug, Error)]
pub enum TextureCreateError {
    #[error("an error has occured: {0}")]
    Other(String),
}

pub struct Texture<B: Backend> {
    ctx: Context<B>,
    dims: (u32, u32, u32),
    pub(crate) id: B::Texture,
}

impl<B: Backend> Texture<B> {
    pub fn new(
        ctx: Context<B>,
        create_info: TextureCreateInfo,
    ) -> Result<Self, TextureCreateError> {
        let dims = (create_info.width, create_info.height, create_info.depth);
        let id = unsafe { ctx.0.create_texture(create_info)? };
        Ok(Self { ctx, dims, id })
    }

    #[inline(always)]
    pub fn dims(&self) -> (u32, u32, u32) {
        self.dims
    }
}

impl<B: Backend> Drop for Texture<B> {
    #[inline(always)]
    fn drop(&mut self) {
        unsafe {
            self.ctx.0.destroy_texture(&mut self.id);
        }
    }
}

impl Default for TextureCreateInfo {
    #[inline(always)]
    fn default() -> Self {
        Self {
            format: TextureFormat::Rgba8Unorm,
            ty: ImageType::TwoDimensions,
            width: 128,
            height: 128,
            depth: 1,
            array_elements: 1,
            mip_levels: 1,
            image_usage: TextureUsage::empty(),
            memory_usage: MemoryUsage::GpuOnly,
            debug_name: None,
        }
    }
}
