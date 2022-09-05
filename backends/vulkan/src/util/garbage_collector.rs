use std::{mem::ManuallyDrop, sync::Mutex};

use ash::vk;
use crossbeam_channel::{Receiver, Sender};
use gpu_allocator::vulkan::{Allocation, Allocator};

pub(crate) struct GarbageCollector {
    sender: Sender<Garbage>,
    receiver: Receiver<Garbage>,
    to_destroy: Mutex<Vec<ToDestroy>>,
}

pub(crate) enum Garbage {
    Pipeline(Vec<vk::Pipeline>, vk::PipelineLayout),
    Buffer {
        buffer: vk::Buffer,
        allocation: Allocation,
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
                Garbage::Pipeline(pipelines, layout) => {
                    device.destroy_pipeline_layout(layout, None);
                    for pipeline in pipelines {
                        device.destroy_pipeline(pipeline, None);
                    }
                }
                Garbage::Buffer { buffer, allocation } => {
                    device.destroy_buffer(buffer, None);
                    allocator.free(allocation).unwrap();
                }
            }
        }
    }
}
