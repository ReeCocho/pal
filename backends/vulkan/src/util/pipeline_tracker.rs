use std::collections::{HashMap, HashSet};

use ash::vk;

#[derive(Default)]
pub(crate) struct PipelineTracker {
    buffers: HashMap<vk::Buffer, ResourceUsage>,
    images: HashMap<vk::Image, ResourceUsage>,
}

#[derive(Copy, Clone)]
pub(crate) struct ResourceUsage {
    /// The pipeline stage where the resource was used.
    pub used: vk::PipelineStageFlags,
    /// The type of memory access that was performed by this operation.
    pub access: vk::AccessFlags,
}

pub(crate) struct Barrier {
    pub src_stage: vk::PipelineStageFlags,
    pub dst_stage: vk::PipelineStageFlags,
    pub dependency: vk::DependencyFlags,
    pub memory_barriers: Vec<vk::MemoryBarrier>,
}

impl PipelineTracker {
    /// Submit with resources and how they are being used. If a dependency is detected from a
    /// previous update, a barrier will be returned that should by applied before the operations
    /// described by the provided update.
    pub fn update(
        &mut self,
        buffers: &[(vk::Buffer, ResourceUsage)],
        images: &[(vk::Image, ResourceUsage)],
    ) -> Option<Barrier> {
        let write_accesses: vk::AccessFlags = vk::AccessFlags::MEMORY_WRITE
            | vk::AccessFlags::COLOR_ATTACHMENT_WRITE
            | vk::AccessFlags::SHADER_WRITE
            | vk::AccessFlags::TRANSFER_WRITE
            | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE;

        let read_accesses: vk::AccessFlags = vk::AccessFlags::INDEX_READ
            | vk::AccessFlags::MEMORY_READ
            | vk::AccessFlags::SHADER_READ
            | vk::AccessFlags::UNIFORM_READ
            | vk::AccessFlags::TRANSFER_READ
            | vk::AccessFlags::COLOR_ATTACHMENT_READ
            | vk::AccessFlags::INDIRECT_COMMAND_READ
            | vk::AccessFlags::VERTEX_ATTRIBUTE_READ
            | vk::AccessFlags::INDEX_READ
            | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ;

        let mut needs_barrier = false;
        let mut memory_barriers = HashMap::<vk::AccessFlags, vk::AccessFlags>::default();
        let mut barrier = Barrier {
            src_stage: vk::PipelineStageFlags::empty(),
            dst_stage: vk::PipelineStageFlags::empty(),
            dependency: vk::DependencyFlags::BY_REGION,
            memory_barriers: Vec::default(),
        };

        for (buffer, usage) in buffers {
            match self.buffers.get_mut(&buffer) {
                Some(old_usage) => {
                    // Read-after-read needs no barrier. Everything else does, however.
                    if !(read_accesses.contains(old_usage.access)
                        && read_accesses.contains(usage.access))
                    {
                        needs_barrier = true;
                        barrier.src_stage |= old_usage.used;
                        barrier.dst_stage |= usage.used;

                        // We only need a memory barrier for read-after-write and write-after-write
                        if !read_accesses.contains(old_usage.access) {
                            let dst_access = memory_barriers.entry(old_usage.access).or_default();
                            *dst_access |= usage.access;
                        }

                        *old_usage = *usage;
                    } else {
                        old_usage.used |= usage.used;
                        old_usage.access |= usage.access;
                    }
                }
                None => {
                    self.buffers.insert(*buffer, *usage);
                }
            }
        }

        for (image, usage) in images {
            match self.images.get_mut(&image) {
                Some(old_usage) => {
                    // Read-after-read needs no barrier. Everything else does, however.
                    if !(read_accesses.contains(old_usage.access)
                        && read_accesses.contains(usage.access))
                    {
                        needs_barrier = true;
                        barrier.src_stage |= old_usage.used;
                        barrier.dst_stage |= usage.used;

                        // We only need a memory barrier for read-after-write and write-after-write
                        if !read_accesses.contains(old_usage.access) {
                            let dst_access = memory_barriers.entry(old_usage.access).or_default();
                            *dst_access |= usage.access;
                        }

                        *old_usage = *usage;
                    } else {
                        old_usage.used |= usage.used;
                        old_usage.access |= usage.access;
                    }
                }
                None => {
                    self.images.insert(*image, *usage);
                }
            }
        }

        if needs_barrier {
            barrier.memory_barriers = memory_barriers
                .into_iter()
                .map(|(src, dst)| {
                    vk::MemoryBarrier::builder()
                        .src_access_mask(src)
                        .dst_access_mask(dst)
                        .build()
                })
                .collect();
            Some(barrier)
        } else {
            None
        }
    }
}

impl Barrier {
    pub unsafe fn execute(&self, device: &ash::Device, command_buffer: vk::CommandBuffer) {
        device.cmd_pipeline_barrier(
            command_buffer,
            self.src_stage,
            self.dst_stage,
            self.dependency,
            &self.memory_barriers,
            &[],
            &[],
        );
    }
}
