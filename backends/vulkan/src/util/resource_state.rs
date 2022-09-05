use api::types::QueueType;
use ash::vk;
use std::collections::HashMap;

use crate::queue::VkQueue;

use super::semaphores::SemaphoreTracker;

#[derive(Default)]
pub(crate) struct ResourceState {
    /// Maps buffer/array element to it's usage.
    buffers: HashMap<(vk::Buffer, usize), LatestUsage>,
}

#[derive(Copy, Clone)]
pub(crate) struct LatestUsage {
    /// The queue type the resource was last used in.
    pub queue: QueueType,
    /// The timeline value when the resource will be done being used.
    pub value: u64,
}

impl ResourceState {
    #[inline(always)]
    pub fn get_buffer(&self, buffer: vk::Buffer, array_elem: usize) -> Option<LatestUsage> {
        self.buffers.get(&(buffer, array_elem)).map(|v| *v)
    }

    #[inline(always)]
    pub fn set_buffer(
        &mut self,
        buffer: vk::Buffer,
        array_elem: usize,
        usage: Option<LatestUsage>,
    ) -> Option<LatestUsage> {
        match usage {
            Some(usage) => self.buffers.insert((buffer, array_elem), usage),
            None => self.buffers.remove(&(buffer, array_elem)),
        }
    }
}

impl LatestUsage {
    pub fn wait_if_needed(
        &self,
        tracker: &mut SemaphoreTracker,
        new_queue: QueueType,
        stage: vk::PipelineStageFlags,
        main: &VkQueue,
        transfer: &VkQueue,
        compute: &VkQueue,
        present: &VkQueue,
    ) {
        if self.queue != new_queue {
            let (semaphore, value) = match self.queue {
                QueueType::Main => (main.semaphore(), main.target_timeline_value()),
                QueueType::Transfer => (transfer.semaphore(), transfer.target_timeline_value()),
                QueueType::Compute => (compute.semaphore(), compute.target_timeline_value()),
                QueueType::Present => (present.semaphore(), present.target_timeline_value()),
            };
            tracker.register_wait(
                semaphore,
                super::semaphores::WaitInfo {
                    value: Some(value),
                    stage,
                },
            );
        }
    }
}
