use std::collections::HashMap;

use ash::vk;

#[derive(Default)]
pub(crate) struct PipelineTracker {
    buffers: HashMap<vk::Buffer, ResourceUsage>,
    images: HashMap<(vk::Image, usize), ResourceUsage>,
}

#[derive(Copy, Clone)]
pub(crate) struct ResourceUsage {
    /// The pipeline stage where the resource was used.
    pub used: vk::PipelineStageFlags,
    /// The type of memory access that was performed by this operation.
    pub access: vk::AccessFlags,
    /// Indicates that an image layout transition is required. Ignored for buffers.
    pub layout_transition: Option<ImageLayoutTransition>,
}

#[derive(Copy, Clone)]
pub(crate) struct ImageLayoutTransition {
    pub old: vk::ImageLayout,
    pub new: vk::ImageLayout,
    pub aspect_mask: vk::ImageAspectFlags,
    pub mip_count: u32,
}

pub(crate) struct Barrier {
    pub src_stage: vk::PipelineStageFlags,
    pub dst_stage: vk::PipelineStageFlags,
    pub dependency: vk::DependencyFlags,
    pub memory_barriers: Vec<vk::MemoryBarrier>,
    pub image_barriers: Vec<vk::ImageMemoryBarrier>,
}

impl PipelineTracker {
    /// Submit with resources and how they are being used. If a dependency is detected from a
    /// previous update, a barrier will be returned that should by applied before the operations
    /// described by the provided update.
    pub fn update(
        &mut self,
        buffers: &[(vk::Buffer, ResourceUsage)],
        images: &[(vk::Image, usize, ResourceUsage)],
    ) -> Option<Barrier> {
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
        let mut image_barriers = HashMap::<
            (vk::Image, usize),
            (ImageLayoutTransition, vk::AccessFlags, vk::AccessFlags),
        >::default();
        let mut barrier = Barrier {
            src_stage: vk::PipelineStageFlags::empty(),
            dst_stage: vk::PipelineStageFlags::empty(),
            dependency: vk::DependencyFlags::BY_REGION,
            memory_barriers: Vec::default(),
            image_barriers: Vec::default(),
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

        for (image, array_element, usage) in images {
            let old_access = match self.images.get_mut(&(*image, *array_element)) {
                Some(old_usage) => {
                    let old_access = old_usage.access;

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

                    old_access
                }
                None => {
                    self.images.insert((*image, *array_element), *usage);
                    vk::AccessFlags::NONE
                }
            };

            // Need a barrier if we are performing a layout transition
            if let Some(transition) = usage.layout_transition {
                needs_barrier = true;

                // Must not already contain the image
                debug_assert!(!image_barriers.contains_key(&(*image, *array_element)));
                image_barriers.insert(
                    (*image, *array_element),
                    (transition, old_access, usage.access),
                );

                // The destination stage must be indicated for when the barrier must be finished.
                barrier.dst_stage |= usage.used;
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
            barrier.image_barriers = image_barriers
                .into_iter()
                .map(
                    |((image, array_elem), (transition, src_access, dst_access))| {
                        vk::ImageMemoryBarrier::builder()
                            .image(image)
                            .old_layout(transition.old)
                            .new_layout(transition.new)
                            .src_access_mask(src_access)
                            .dst_access_mask(dst_access)
                            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                            .subresource_range(
                                vk::ImageSubresourceRange::builder()
                                    .aspect_mask(transition.aspect_mask)
                                    .base_mip_level(0)
                                    .level_count(transition.mip_count)
                                    .base_array_layer(array_elem as u32)
                                    .layer_count(1)
                                    .build(),
                            )
                            .build()
                    },
                )
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
            match self.src_stage {
                vk::PipelineStageFlags::NONE => vk::PipelineStageFlags::TOP_OF_PIPE,
                other => other,
            },
            self.dst_stage,
            self.dependency,
            &self.memory_barriers,
            &[],
            &self.image_barriers,
        );
    }
}
