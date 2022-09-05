use std::collections::HashMap;

use api::descriptor_set::DescriptorSetLayoutCreateInfo;
use ash::vk;

/// Default number of sets per pool.
const SETS_PER_POOL: usize = 16;

pub(crate) struct DescriptorPool {
    /// # Note
    /// The layout is held in an array for convenience when allocating pools.
    layout: [vk::DescriptorSetLayout; 1],
    /// Pools to allocate sets from.
    pools: Vec<vk::DescriptorPool>,
    /// Current number of sets allocated from the top pool.
    size: usize,
    /// Free list of descriptor sets.
    free: Vec<vk::DescriptorSet>,
    /// Pool sizes to use when making a new descriptor pool.
    sizes: Vec<vk::DescriptorPoolSize>,
}

impl DescriptorPool {
    pub unsafe fn new(
        device: &ash::Device,
        create_info: &DescriptorSetLayoutCreateInfo<crate::VulkanBackend>,
    ) -> Self {
        // Convert the api layout into a vulkan layout
        let mut bindings = Vec::default();
        for binding in &create_info.bindings {
            bindings.push(
                vk::DescriptorSetLayoutBinding::builder()
                    .binding(binding.binding)
                    .descriptor_count(binding.count as u32)
                    .descriptor_type(super::to_vk_descriptor_type(binding.ty))
                    .build(),
            );
        }

        let create_info = vk::DescriptorSetLayoutCreateInfo::builder()
            .bindings(&bindings)
            .build();

        // Create the layout
        let layout = device
            .create_descriptor_set_layout(&create_info, None)
            .unwrap();

        // Create the required pool sizes
        // Maps descriptor types to the number required
        let mut pool_sizes = HashMap::<vk::DescriptorType, u32>::default();
        for i in 0..create_info.binding_count {
            let binding = create_info.p_bindings.add(i as usize).as_ref().unwrap();

            // Update pool size
            *pool_sizes.entry(binding.descriptor_type).or_default() += binding.descriptor_count;
        }

        // Convert map of pool sizes into a vec
        let sizes = pool_sizes
            .into_iter()
            .map(|(ty, count)| vk::DescriptorPoolSize {
                ty,
                descriptor_count: count * SETS_PER_POOL as u32,
            })
            .collect::<Vec<_>>();

        Self {
            layout: [layout],
            pools: Vec::default(),
            size: 0,
            free: Vec::default(),
            sizes,
        }
    }
}
