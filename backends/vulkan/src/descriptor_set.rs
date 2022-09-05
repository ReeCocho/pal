use api::descriptor_set::{DescriptorBinding, DescriptorSetLayoutCreateInfo};
use ash::vk;
use crossbeam_channel::Sender;

use crate::{buffer::BufferRefCounter, util::garbage_collector::Garbage};

pub struct DescriptorSet {
    pub(crate) set: vk::DescriptorSet,
    pub(crate) layout: vk::DescriptorSetLayout,
    pub(crate) bound: Vec<Vec<Option<Binding>>>,
    pub(crate) on_drop: Sender<Garbage>,
}

pub struct DescriptorSetLayout {
    pub(crate) descriptor: DescriptorSetLayoutCreateInfo,
    pub(crate) layout: vk::DescriptorSetLayout,
}

pub(crate) struct Binding {
    pub value: BoundValue,
    pub stage: vk::PipelineStageFlags,
    pub access: vk::AccessFlags,
}

pub(crate) enum BoundValue {
    UniformBuffer {
        ref_counter: BufferRefCounter,
        buffer: vk::Buffer,
        array_element: usize,
    },
    StorageBuffer {
        ref_counter: BufferRefCounter,
        buffer: vk::Buffer,
        array_element: usize,
    },
}

impl DescriptorSetLayout {
    #[inline]
    pub(crate) fn get_binding(&self, binding_value: u32) -> Option<&DescriptorBinding> {
        for binding in &self.descriptor.bindings {
            if binding.binding == binding_value {
                return Some(binding);
            }
        }
        None
    }
}

impl Drop for DescriptorSet {
    fn drop(&mut self) {
        self.on_drop
            .send(Garbage::DescriptorSet {
                set: self.set,
                layout: self.layout,
            })
            .unwrap();
    }
}
