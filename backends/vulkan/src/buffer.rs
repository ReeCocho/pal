use std::mem::ManuallyDrop;

use api::{
    buffer::{BufferCreateError, BufferCreateInfo},
    types::{BufferUsage, MemoryUsage},
};
use ash::vk;
use crossbeam_channel::Sender;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, Allocator};

use crate::util::garbage_collector::Garbage;

pub struct Buffer {
    pub(crate) buffer: vk::Buffer,
    pub(crate) block: ManuallyDrop<Allocation>,
    pub(crate) buffer_usage: BufferUsage,
    pub(crate) memory_usage: MemoryUsage,
    pub(crate) array_elements: usize,
    /// This was the user requested size of each array element.
    pub(crate) size: u64,
    /// This is the per element size after alignment.
    pub(crate) aligned_size: u64,
    on_drop: Sender<Garbage>,
}

impl Buffer {
    pub(crate) unsafe fn new(
        device: &ash::Device,
        on_drop: Sender<Garbage>,
        allocator: &mut Allocator,
        limits: &vk::PhysicalDeviceLimits,
        create_info: BufferCreateInfo,
    ) -> Result<Self, BufferCreateError> {
        // Determine memory alignment requirements
        let mut alignment_req = 0;
        if create_info.memory_usage == MemoryUsage::CpuToGpu {
            alignment_req = alignment_req.max(limits.non_coherent_atom_size);
        }
        if create_info
            .buffer_usage
            .contains(BufferUsage::UNIFORM_BUFFER)
        {
            alignment_req = alignment_req.max(limits.min_uniform_buffer_offset_alignment);
        }
        if create_info
            .buffer_usage
            .contains(BufferUsage::STORAGE_BUFFER)
        {
            alignment_req = alignment_req.max(limits.min_storage_buffer_offset_alignment);
        }

        // Round size to a multiple of the alignment
        let aligned_size = match alignment_req {
            0 => create_info.size,
            align => {
                let align_mask = align - 1;
                (create_info.size + align_mask) & !align_mask
            }
        };

        // Create the buffer
        let buffer_create_info = vk::BufferCreateInfo::builder()
            .size(aligned_size * create_info.array_elements as u64)
            .usage(crate::util::to_vk_buffer_usage(create_info.buffer_usage))
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .build();
        let buffer = match device.create_buffer(&buffer_create_info, None) {
            Ok(buffer) => buffer,
            Err(err) => return Err(BufferCreateError::Other(err.to_string())),
        };

        // Allocate memory
        let mem_reqs = device.get_buffer_memory_requirements(buffer);
        let request = AllocationCreateDesc {
            name: "buffer",
            requirements: mem_reqs,
            location: crate::util::to_gpu_allocator_memory_location(create_info.memory_usage),
            linear: true,
        };
        let block = match allocator.allocate(&request) {
            Ok(block) => block,
            Err(err) => {
                device.destroy_buffer(buffer, None);
                return Err(BufferCreateError::Other(err.to_string()));
            }
        };

        // Bind buffer to memory
        if let Err(err) = device.bind_buffer_memory(buffer, block.memory(), block.offset()) {
            allocator.free(block).unwrap();
            device.destroy_buffer(buffer, None);
            return Err(BufferCreateError::Other(err.to_string()));
        }

        Ok(Buffer {
            buffer,
            block: ManuallyDrop::new(block),
            size: create_info.size,
            aligned_size,
            array_elements: create_info.array_elements,
            buffer_usage: create_info.buffer_usage,
            memory_usage: create_info.memory_usage,
            on_drop,
        })
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.on_drop
            .send(Garbage::Buffer {
                buffer: self.buffer,
                allocation: unsafe { ManuallyDrop::take(&mut self.block) },
            })
            .unwrap();
    }
}
