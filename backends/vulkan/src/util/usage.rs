use std::collections::{hash_map::Iter, HashMap};

use api::types::QueueType;
use ash::vk;
use fxhash::FxHashMap;

use super::fast_int_hasher::FIHashMap;

/// This keeps track of which queues and when buffers and images are used in. Additionally, it
/// keeps track of the layouts of images on a per-mip level.
#[derive(Default)]
pub(crate) struct GlobalResourceUsage {
    sets: FIHashMap<vk::DescriptorSet, QueueUsage>,
    /// Buffer + array element.
    buffers: FxHashMap<(vk::Buffer, u32), QueueUsage>,
    /// Texture + array element.
    images: FxHashMap<(vk::Image, u32), QueueUsage>,
    /// Texture + array element + mip level.
    image_layouts: FxHashMap<(vk::Image, u32, u32), vk::ImageLayout>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct QueueUsage {
    pub queue: QueueType,
    pub timeline_value: u64,
}

pub(crate) struct PipelineTracker<'a> {
    global: &'a mut GlobalResourceUsage,
    queue_ty: QueueType,
    next_value: u64,
    usages: FxHashMap<SubResource, SubResourceUsage>,
    queues: FIHashMap<QueueType, vk::PipelineStageFlags>,
}

#[derive(Default)]
pub(crate) struct UsageScope {
    usages: FxHashMap<SubResource, SubResourceUsage>,
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SubResourceUsage {
    pub access: vk::AccessFlags,
    pub stage: vk::PipelineStageFlags,
    /// Unused by buffers.
    pub layout: vk::ImageLayout,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum SubResource {
    Buffer {
        buffer: vk::Buffer,
        array_elem: u32,
    },
    Texture {
        texture: vk::Image,
        aspect_mask: vk::ImageAspectFlags,
        array_elem: u32,
        mip_level: u32,
    },
}

#[derive(Default)]
pub(crate) struct PipelineBarrier {
    pub src_stage: vk::PipelineStageFlags,
    pub dst_stage: vk::PipelineStageFlags,
    pub buffer_barriers: Vec<vk::BufferMemoryBarrier>,
    pub image_barriers: Vec<vk::ImageMemoryBarrier>,
}

impl GlobalResourceUsage {
    #[inline(always)]
    pub fn register_set(
        &mut self,
        set: vk::DescriptorSet,
        usage: Option<QueueUsage>,
    ) -> Option<QueueUsage> {
        match usage {
            Some(usage) => self.sets.insert(set, usage),
            None => self.sets.remove(&set),
        }
    }

    #[inline(always)]
    pub fn register_buffer(
        &mut self,
        buffer: vk::Buffer,
        array_elem: u32,
        usage: Option<QueueUsage>,
    ) -> Option<QueueUsage> {
        match usage {
            Some(usage) => self.buffers.insert((buffer, array_elem), usage),
            None => self.buffers.remove(&(buffer, array_elem)),
        }
    }

    #[inline(always)]
    pub fn register_image(
        &mut self,
        image: vk::Image,
        array_elem: u32,
        usage: Option<QueueUsage>,
    ) -> Option<QueueUsage> {
        match usage {
            Some(usage) => self.images.insert((image, array_elem), usage),
            None => self.images.remove(&(image, array_elem)),
        }
    }

    #[inline(always)]
    pub fn register_layout(
        &mut self,
        image: vk::Image,
        array_elem: u32,
        mip_level: u32,
        layout: vk::ImageLayout,
    ) -> vk::ImageLayout {
        self.image_layouts
            .insert((image, array_elem, mip_level), layout)
            .unwrap_or_default()
    }
}

impl<'a> PipelineTracker<'a> {
    #[inline(always)]
    pub fn new(global: &'a mut GlobalResourceUsage, queue_ty: QueueType, next_value: u64) -> Self {
        Self {
            global,
            queue_ty,
            next_value,
            usages: HashMap::default(),
            queues: HashMap::default(),
        }
    }

    pub fn submit(&mut self, scope: UsageScope) -> Option<PipelineBarrier> {
        let read_accesses: vk::AccessFlags = vk::AccessFlags::MEMORY_READ
            | vk::AccessFlags::SHADER_READ
            | vk::AccessFlags::UNIFORM_READ
            | vk::AccessFlags::TRANSFER_READ
            | vk::AccessFlags::COLOR_ATTACHMENT_READ
            | vk::AccessFlags::INDIRECT_COMMAND_READ
            | vk::AccessFlags::VERTEX_ATTRIBUTE_READ
            | vk::AccessFlags::INDEX_READ
            | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ;

        let mut barrier = PipelineBarrier::default();

        // Keeps track of which image subresources need memor barriers and/or layout transitions
        let mut image_barriers =
            FxHashMap::<(vk::Image, u32, u32), vk::ImageMemoryBarrier>::default();

        // Keeps track of which buffers need memory barriers
        let mut buffer_barriers =
            FxHashMap::<(vk::Buffer, u32), vk::BufferMemoryBarrier>::default();

        // Analyze each usage
        for (resource, usage) in scope.usages {
            // Check the global tracker to see if we need to wait on certain queues or if we need
            // a layout transition.
            let resc_usage = QueueUsage {
                queue: self.queue_ty,
                timeline_value: self.next_value,
            };
            let (old_queue_usage, old_layout) = match &resource {
                SubResource::Buffer { buffer, array_elem } => (
                    self.global
                        .register_buffer(*buffer, *array_elem, Some(resc_usage)),
                    vk::ImageLayout::UNDEFINED,
                ),
                SubResource::Texture {
                    texture,
                    array_elem,
                    mip_level,
                    ..
                } => (
                    self.global
                        .register_image(*texture, *array_elem, Some(resc_usage)),
                    self.global
                        .register_layout(*texture, *array_elem, *mip_level, usage.layout),
                ),
            };

            // Check if this resource was last used by a queue other than us
            if let Some(old_queue_usage) = old_queue_usage {
                if old_queue_usage.queue != self.queue_ty {
                    // The new usage might be a combo of a bunch of other usages, so we have to
                    // select them in order. This is OMEGA ugly. Look into changing this
                    let new_stage = if usage.stage.contains(vk::PipelineStageFlags::BOTTOM_OF_PIPE)
                    {
                        vk::PipelineStageFlags::BOTTOM_OF_PIPE
                    } else if usage.stage.contains(vk::PipelineStageFlags::COMPUTE_SHADER) {
                        vk::PipelineStageFlags::COMPUTE_SHADER
                    } else if usage.stage.contains(vk::PipelineStageFlags::TRANSFER) {
                        vk::PipelineStageFlags::TRANSFER
                    } else if usage
                        .stage
                        .contains(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                    {
                        vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    } else if usage
                        .stage
                        .contains(vk::PipelineStageFlags::LATE_FRAGMENT_TESTS)
                    {
                        vk::PipelineStageFlags::LATE_FRAGMENT_TESTS
                    } else if usage
                        .stage
                        .contains(vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS)
                    {
                        vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                    } else if usage
                        .stage
                        .contains(vk::PipelineStageFlags::FRAGMENT_SHADER)
                    {
                        vk::PipelineStageFlags::FRAGMENT_SHADER
                    } else if usage
                        .stage
                        .contains(vk::PipelineStageFlags::GEOMETRY_SHADER)
                    {
                        vk::PipelineStageFlags::GEOMETRY_SHADER
                    } else if usage
                        .stage
                        .contains(vk::PipelineStageFlags::TESSELLATION_EVALUATION_SHADER)
                    {
                        vk::PipelineStageFlags::TESSELLATION_EVALUATION_SHADER
                    } else if usage
                        .stage
                        .contains(vk::PipelineStageFlags::TESSELLATION_CONTROL_SHADER)
                    {
                        vk::PipelineStageFlags::TESSELLATION_CONTROL_SHADER
                    } else if usage.stage.contains(vk::PipelineStageFlags::VERTEX_SHADER) {
                        vk::PipelineStageFlags::VERTEX_SHADER
                    } else if usage.stage.contains(vk::PipelineStageFlags::VERTEX_INPUT) {
                        vk::PipelineStageFlags::VERTEX_INPUT
                    } else if usage.stage.contains(vk::PipelineStageFlags::DRAW_INDIRECT) {
                        vk::PipelineStageFlags::DRAW_INDIRECT
                    } else {
                        vk::PipelineStageFlags::TOP_OF_PIPE
                    };
                    let entry = self
                        .queues
                        .entry(old_queue_usage.queue)
                        .or_insert(vk::PipelineStageFlags::BOTTOM_OF_PIPE);
                    if crate::util::rank_pipeline_stage(new_stage)
                        < crate::util::rank_pipeline_stage(*entry)
                    {
                        *entry = new_stage;
                    }
                }
            }

            // Barrier is required if we have mismatching image layouts
            let mut needs_barrier = old_layout != usage.layout;
            let (src_access, src_stage) = match self.usages.get_mut(&resource) {
                Some(old) => {
                    // Anything other than read-after-read requires a barrier
                    if !(read_accesses.contains(old.access) && read_accesses.contains(usage.access))
                    {
                        needs_barrier = true;
                    }
                    (old.access, old.stage)
                }
                // If there was no previous usage, no barrier is needed
                None => (vk::AccessFlags::NONE, vk::PipelineStageFlags::TOP_OF_PIPE),
            };
            self.usages.insert(resource, usage);

            if needs_barrier {
                match resource {
                    SubResource::Buffer { buffer, array_elem } => {
                        buffer_barriers.insert(
                            (buffer, array_elem),
                            vk::BufferMemoryBarrier::builder()
                                .src_access_mask(src_access)
                                .dst_access_mask(usage.access)
                                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                                .buffer(buffer)
                                .offset(0)
                                .size(vk::WHOLE_SIZE)
                                .build(),
                        );
                    }
                    SubResource::Texture {
                        texture,
                        array_elem,
                        mip_level,
                        aspect_mask,
                    } => {
                        image_barriers.insert(
                            (texture, array_elem, mip_level),
                            vk::ImageMemoryBarrier::builder()
                                .src_access_mask(src_access)
                                .dst_access_mask(usage.access)
                                .old_layout(old_layout)
                                .new_layout(usage.layout)
                                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                                .image(texture)
                                .subresource_range(vk::ImageSubresourceRange {
                                    aspect_mask: aspect_mask,
                                    base_mip_level: mip_level,
                                    level_count: 1,
                                    base_array_layer: array_elem,
                                    layer_count: 1,
                                })
                                .build(),
                        );
                    }
                }

                // Update barrier with stages
                barrier.dst_stage |= usage.stage;
                barrier.src_stage |= src_stage;
            }
        }

        // We only need a barrier if we have registered buffer/image barriers
        if !image_barriers.is_empty() || !buffer_barriers.is_empty() {
            barrier.image_barriers = image_barriers.into_iter().map(|(_, v)| v).collect();
            barrier.buffer_barriers = buffer_barriers.into_iter().map(|(_, v)| v).collect();
            Some(barrier)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn wait_queues(&self) -> Iter<'_, QueueType, vk::PipelineStageFlags> {
        self.queues.iter()
    }
}

impl UsageScope {
    #[inline(always)]
    pub fn use_resource(&mut self, subresource: SubResource, usage: SubResourceUsage) {
        let mut entry = self.usages.entry(subresource).or_default();
        assert!(
            entry.layout == vk::ImageLayout::UNDEFINED || entry.layout == usage.layout,
            "an image can only have one layout per scope"
        );
        entry.layout = usage.layout;
        entry.access |= usage.access;
        entry.stage |= usage.stage;
    }
}

impl PipelineBarrier {
    #[inline(always)]
    pub unsafe fn execute(&self, device: &ash::Device, command_buffer: vk::CommandBuffer) {
        device.cmd_pipeline_barrier(
            command_buffer,
            self.src_stage,
            self.dst_stage,
            vk::DependencyFlags::BY_REGION,
            &[],
            &self.buffer_barriers,
            &self.image_barriers,
        );
    }
}
