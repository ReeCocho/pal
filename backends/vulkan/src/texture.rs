use std::{ffi::CString, mem::ManuallyDrop, sync::Arc};

use crate::util::garbage_collector::Garbage;
use api::{
    texture::{TextureCreateError, TextureCreateInfo},
    types::*,
};
use ash::vk::{self, Handle};
use crossbeam_channel::Sender;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, Allocator};

pub struct Texture {
    pub(crate) image: vk::Image,
    pub(crate) block: ManuallyDrop<Allocation>,
    pub(crate) image_usage: TextureUsage,
    pub(crate) memory_usage: MemoryUsage,
    pub(crate) array_elements: usize,
    pub(crate) size: u64,
    pub(crate) ref_counter: TextureRefCounter,
    on_drop: Sender<Garbage>,
}

#[derive(Clone)]
pub(crate) struct TextureRefCounter(Arc<()>);

impl Texture {
    pub(crate) unsafe fn new(
        device: &ash::Device,
        debug: Option<&ash::extensions::ext::DebugUtils>,
        on_drop: Sender<Garbage>,
        allocator: &mut Allocator,
        create_info: TextureCreateInfo,
    ) -> Result<Self, TextureCreateError> {
        // Create the image
        let image_create_info = vk::ImageCreateInfo::builder()
            .image_type(crate::util::to_vk_image_type(create_info.ty))
            .extent(vk::Extent3D {
                width: create_info.width,
                height: create_info.height,
                depth: create_info.depth,
            })
            .mip_levels(create_info.mip_levels as u32)
            .array_layers(create_info.array_elements as u32)
            .format(crate::util::to_vk_format(create_info.format))
            .tiling(vk::ImageTiling::OPTIMAL)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .usage(crate::util::to_vk_image_usage(create_info.image_usage))
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .samples(vk::SampleCountFlags::TYPE_1)
            .flags(
                if create_info.array_elements % 6 == 0
                    && create_info.width == create_info.height
                    && create_info.ty == ImageType::TwoDimensions
                {
                    vk::ImageCreateFlags::CUBE_COMPATIBLE
                } else {
                    vk::ImageCreateFlags::empty()
                },
            )
            .build();

        let image = match device.create_image(&image_create_info, None) {
            Ok(image) => image,
            Err(err) => return Err(TextureCreateError::Other(err.to_string())),
        };

        // Determine memory requirements
        let mem_reqs = device.get_image_memory_requirements(image);

        // Allocate memory
        let request = AllocationCreateDesc {
            name: match &create_info.debug_name {
                Some(name) => &name,
                None => "image",
            },
            requirements: mem_reqs,
            location: crate::util::to_gpu_allocator_memory_location(create_info.memory_usage),
            linear: false,
        };

        let block = match allocator.allocate(&request) {
            Ok(block) => block,
            Err(err) => {
                device.destroy_image(image, None);
                return Err(TextureCreateError::Other(err.to_string()));
            }
        };

        // Bind image to memory
        if let Err(err) = device.bind_image_memory(image, block.memory(), block.offset()) {
            allocator.free(block).unwrap();
            device.destroy_image(image, None);
            return Err(TextureCreateError::Other(err.to_string()));
        }

        // Setup debug name is requested
        if let Some(name) = create_info.debug_name {
            if let Some(debug) = debug {
                let name = CString::new(name).unwrap();
                let name_info = vk::DebugUtilsObjectNameInfoEXT::builder()
                    .object_type(vk::ObjectType::IMAGE)
                    .object_handle(image.as_raw())
                    .object_name(&name)
                    .build();

                debug
                    .debug_utils_set_object_name(device.handle(), &name_info)
                    .unwrap();
            }
        }

        Ok(Texture {
            image,
            block: ManuallyDrop::new(block),
            image_usage: create_info.image_usage,
            memory_usage: create_info.memory_usage,
            array_elements: create_info.array_elements,
            size: mem_reqs.size,
            on_drop,
            ref_counter: TextureRefCounter::default(),
        })
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        self.on_drop
            .send(Garbage::Texture {
                image: self.image,
                allocation: unsafe { ManuallyDrop::take(&mut self.block) },
                ref_counter: self.ref_counter.clone(),
            })
            .unwrap();
    }
}

impl TextureRefCounter {
    #[inline]
    pub fn is_last(&self) -> bool {
        Arc::strong_count(&self.0) == 1
    }
}

impl Default for TextureRefCounter {
    #[inline]
    fn default() -> Self {
        TextureRefCounter(Arc::new(()))
    }
}
