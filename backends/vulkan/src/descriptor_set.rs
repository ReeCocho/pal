use api::{
    descriptor_set::{
        DescriptorBinding, DescriptorSetCreateError, DescriptorSetCreateInfo,
        DescriptorSetLayoutCreateError, DescriptorSetLayoutCreateInfo, DescriptorSetUpdate,
        DescriptorType, DescriptorValue,
    },
    types::{AccessType, ShaderStage},
};
use ash::vk;
use crossbeam_channel::Sender;

use crate::{
    buffer::BufferRefCounter,
    util::{descriptor_pool::DescriptorPools, garbage_collector::Garbage},
};

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
        _ref_counter: BufferRefCounter,
        buffer: vk::Buffer,
        array_element: usize,
    },
    StorageBuffer {
        _ref_counter: BufferRefCounter,
        buffer: vk::Buffer,
        array_element: usize,
    },
}

impl DescriptorSetLayout {
    pub(crate) unsafe fn new(
        device: &ash::Device,
        pools: &mut DescriptorPools,
        create_info: DescriptorSetLayoutCreateInfo,
    ) -> Result<Self, DescriptorSetLayoutCreateError> {
        // Pre-cache the pool
        let pool = pools.get(device, create_info.clone());
        Ok(DescriptorSetLayout {
            descriptor: create_info,
            layout: pool.layout(),
        })
    }

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

impl DescriptorSet {
    pub(crate) unsafe fn new(
        device: &ash::Device,
        pools: &mut DescriptorPools,
        garbage: Sender<Garbage>,
        debug: Option<&ash::extensions::ext::DebugUtils>,
        create_info: DescriptorSetCreateInfo<crate::VulkanBackend>,
    ) -> Result<Self, DescriptorSetCreateError> {
        let mut bound = Vec::with_capacity(create_info.layout.internal().descriptor.bindings.len());
        for binding in &create_info.layout.internal().descriptor.bindings {
            let mut binds = Vec::with_capacity(binding.count);
            binds.resize_with(binding.count, || None);
            bound.push(binds);
        }

        let pool = pools.get(device, create_info.layout.internal().descriptor.clone());
        let set = pool.allocate(device, debug, create_info.debug_name);
        Ok(DescriptorSet {
            set,
            layout: pool.layout(),
            on_drop: garbage,
            bound,
        })
    }

    pub(crate) unsafe fn update(
        &mut self,
        device: &ash::Device,
        layout: &DescriptorSetLayout,
        updates: &[DescriptorSetUpdate<crate::VulkanBackend>],
    ) {
        let mut writes = Vec::with_capacity(updates.len());
        let mut buffers = Vec::with_capacity(updates.len());

        for update in updates {
            self.bound[update.binding as usize][update.array_element] = Some({
                let binding = layout.get_binding(update.binding).unwrap();
                let access = match binding.ty {
                    DescriptorType::Texture => vk::AccessFlags::SHADER_READ,
                    DescriptorType::UniformBuffer => vk::AccessFlags::UNIFORM_READ,
                    DescriptorType::StorageBuffer(ty) => match ty {
                        AccessType::Read => vk::AccessFlags::SHADER_READ,
                        AccessType::ReadWrite => {
                            vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE
                        }
                    },
                };
                let stage = match binding.stage {
                    ShaderStage::Vertex => vk::PipelineStageFlags::VERTEX_SHADER,
                    ShaderStage::Fragment => vk::PipelineStageFlags::FRAGMENT_SHADER,
                    ShaderStage::Compute => vk::PipelineStageFlags::COMPUTE_SHADER,
                    ShaderStage::AllGraphics => vk::PipelineStageFlags::ALL_GRAPHICS,
                };

                match &update.value {
                    DescriptorValue::UniformBuffer {
                        buffer,
                        array_element,
                    } => {
                        let buffer = buffer.internal();
                        buffers.push(
                            vk::DescriptorBufferInfo::builder()
                                .buffer(buffer.buffer)
                                .offset(buffer.aligned_size * (*array_element) as u64)
                                .range(buffer.aligned_size)
                                .build(),
                        );

                        writes.push(
                            vk::WriteDescriptorSet::builder()
                                .dst_set(self.set)
                                .dst_binding(update.binding)
                                .dst_array_element(update.array_element as u32)
                                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                                .buffer_info(&buffers[buffers.len() - 1..])
                                .build(),
                        );

                        Binding {
                            access,
                            stage,
                            value: BoundValue::UniformBuffer {
                                _ref_counter: buffer.ref_counter.clone(),
                                buffer: buffer.buffer,
                                array_element: *array_element,
                            },
                        }
                    }
                    DescriptorValue::StorageBuffer {
                        buffer,
                        array_element,
                    } => {
                        let buffer = buffer.internal();
                        buffers.push(
                            vk::DescriptorBufferInfo::builder()
                                .buffer(buffer.buffer)
                                .offset(buffer.aligned_size * (*array_element) as u64)
                                .range(buffer.aligned_size)
                                .build(),
                        );

                        writes.push(
                            vk::WriteDescriptorSet::builder()
                                .dst_set(self.set)
                                .dst_binding(update.binding)
                                .dst_array_element(update.array_element as u32)
                                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                                .buffer_info(&buffers[buffers.len() - 1..])
                                .build(),
                        );

                        Binding {
                            access,
                            stage,
                            value: BoundValue::StorageBuffer {
                                _ref_counter: buffer.ref_counter.clone(),
                                buffer: buffer.buffer,
                                array_element: *array_element,
                            },
                        }
                    }
                    DescriptorValue::Texture => todo!(),
                }
            });
        }

        device.update_descriptor_sets(&writes, &[]);
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
