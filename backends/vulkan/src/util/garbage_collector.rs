use std::sync::Mutex;

use ash::vk;
use crossbeam_channel::{Receiver, Sender};
use gpu_allocator::vulkan::{Allocation, Allocator};

use crate::buffer::BufferRefCounter;

use super::{descriptor_pool::DescriptorPools, pipeline_cache::PipelineCache};

pub(crate) struct GarbageCollector {
    sender: Sender<Garbage>,
    receiver: Receiver<Garbage>,
    to_destroy: Mutex<Vec<ToDestroy>>,
}

pub(crate) enum Garbage {
    PipelineLayout(vk::PipelineLayout),
    Pipeline(vk::Pipeline),
    Buffer {
        buffer: vk::Buffer,
        allocation: Allocation,
        ref_counter: BufferRefCounter,
    },
    DescriptorSet {
        set: vk::DescriptorSet,
        layout: vk::DescriptorSetLayout,
    },
}

#[derive(Copy, Clone)]
pub(crate) struct TimelineValues {
    pub main: u64,
    pub transfer: u64,
    pub compute: u64,
}

struct ToDestroy {
    garbage: Garbage,
    values: TimelineValues,
}

impl GarbageCollector {
    pub fn new() -> Self {
        let (sender, receiver) = crossbeam_channel::unbounded();
        Self {
            sender,
            receiver,
            to_destroy: Mutex::new(Vec::default()),
        }
    }

    pub fn sender(&self) -> Sender<Garbage> {
        self.sender.clone()
    }

    pub unsafe fn cleanup(
        &self,
        device: &ash::Device,
        allocator: &mut Allocator,
        pools: &mut DescriptorPools,
        pipelines: &mut PipelineCache,
        current: TimelineValues,
        target: TimelineValues,
    ) {
        // Receive all incoming garbage
        let mut to_destroy = self.to_destroy.lock().unwrap();
        while let Ok(garbage) = self.receiver.try_recv() {
            to_destroy.push(ToDestroy {
                garbage,
                values: target,
            });
        }

        // Mark everything that is not being used by any queue
        let mut marked = Vec::default();
        for (i, garbage) in to_destroy.iter().enumerate() {
            match &garbage.garbage {
                Garbage::Buffer { ref_counter, .. } => {
                    if !ref_counter.is_last() {
                        continue;
                    }
                }
                _ => {}
            }

            if garbage.values.main <= current.main
                && garbage.values.transfer <= current.transfer
                && garbage.values.compute <= current.compute
            {
                marked.push(i);
            }
        }

        // Remove marked elements from the list
        marked.sort_unstable();
        for i in marked.into_iter().rev() {
            match to_destroy.remove(i).garbage {
                Garbage::PipelineLayout(layout) => {
                    // Also destroy associated pipelines
                    pipelines.release(device, layout);
                    device.destroy_pipeline_layout(layout, None);
                }
                Garbage::Pipeline(pipeline) => {
                    device.destroy_pipeline(pipeline, None);
                }
                Garbage::Buffer {
                    buffer, allocation, ..
                } => {
                    device.destroy_buffer(buffer, None);
                    allocator.free(allocation).unwrap();
                }
                Garbage::DescriptorSet { set, layout } => {
                    pools.get_by_layout(layout).unwrap().free(set);
                }
            }
        }
    }
}
