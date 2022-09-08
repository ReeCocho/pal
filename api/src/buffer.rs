use std::ptr::NonNull;

use crate::{context::Context, types::*, Backend};
use thiserror::Error;

pub struct BufferCreateInfo {
    pub size: u64,
    pub array_elements: usize,
    pub buffer_usage: BufferUsage,
    pub memory_usage: MemoryUsage,
    pub debug_name: Option<String>,
}

#[derive(Debug, Error)]
pub enum BufferCreateError {
    #[error("an error has occured: {0}")]
    Other(String),
}

#[derive(Debug, Error)]
pub enum BufferViewError {
    #[error("this buffer is not mappable")]
    NotMapable,
    #[error("an error has occured: {0}")]
    Other(String),
}

pub struct Buffer<B: Backend> {
    ctx: Context<B>,
    size: u64,
    pub(crate) id: B::Buffer,
}

pub struct BufferReadView<'a, B: Backend> {
    buffer: &'a Buffer<B>,
    map: NonNull<u8>,
    len: u64,
}

pub struct BufferWriteView<'a, B: Backend> {
    ctx: Context<B>,
    idx: usize,
    buffer: &'a mut Buffer<B>,
    map: NonNull<u8>,
    len: u64,
}

impl<B: Backend> Buffer<B> {
    #[inline(always)]
    pub fn new(ctx: Context<B>, create_info: BufferCreateInfo) -> Result<Self, BufferCreateError> {
        let size = create_info.size;
        let id = unsafe { ctx.0.create_buffer(create_info)? };
        Ok(Self { ctx, id, size })
    }

    pub fn new_staging(
        ctx: Context<B>,
        debug_name: Option<String>,
        data: &[u8],
    ) -> Result<Buffer<B>, BufferCreateError> {
        let create_info = BufferCreateInfo {
            size: data.len() as u64,
            array_elements: 1,
            buffer_usage: BufferUsage::TRANSFER_SRC,
            memory_usage: MemoryUsage::CpuToGpu,
            debug_name,
        };
        let mut buffer = Buffer::new(ctx, create_info)?;
        let mut view = buffer.write(0).unwrap();
        view.as_slice_mut().copy_from_slice(&data);
        std::mem::drop(view);
        Ok(buffer)
    }

    #[inline(always)]
    pub fn internal(&self) -> &B::Buffer {
        &self.id
    }

    #[inline(always)]
    pub fn size(&self) -> u64 {
        self.size
    }

    #[inline(always)]
    pub fn read(&mut self, idx: usize) -> Result<BufferReadView<B>, BufferViewError> {
        let (map, len) = unsafe {
            let res = self.ctx.0.map_memory(&mut self.id, idx)?;
            self.ctx.0.invalidate_range(&mut self.id, idx);
            res
        };
        Ok(BufferReadView {
            buffer: self,
            map,
            len,
        })
    }

    #[inline(always)]
    pub fn write(&mut self, idx: usize) -> Result<BufferWriteView<B>, BufferViewError> {
        let (map, len) = unsafe {
            let res = self.ctx.0.map_memory(&mut self.id, idx)?;
            self.ctx.0.invalidate_range(&mut self.id, idx);
            res
        };
        Ok(BufferWriteView {
            idx,
            ctx: self.ctx.clone(),
            buffer: self,
            map,
            len,
        })
    }
}

impl<B: Backend> Drop for Buffer<B> {
    fn drop(&mut self) {
        unsafe {
            self.ctx.0.destroy_buffer(&mut self.id);
        }
    }
}

impl<'a, B: Backend> BufferReadView<'a, B> {
    #[inline(always)]
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.map.as_ptr(), self.len as usize) }
    }
}

impl<'a, B: Backend> BufferWriteView<'a, B> {
    #[inline(always)]
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.map.as_ptr(), self.len as usize) }
    }

    #[inline(always)]
    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.map.as_ptr(), self.len as usize) }
    }
}

impl<'a, B: Backend> Drop for BufferWriteView<'a, B> {
    fn drop(&mut self) {
        unsafe {
            self.ctx.0.flush_range(&mut self.buffer.id, self.idx);
        }
    }
}
