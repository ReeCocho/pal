use std::sync::atomic::{AtomicBool, Ordering};

use api::surface::{SurfaceConfiguration, SurfaceImageAcquireError, SurfaceUpdateError};
use ash::vk;

use crate::VulkanBackend;

pub struct SurfaceId(pub(crate) usize);

pub struct Surface {
    pub(crate) surface: vk::SurfaceKHR,
    pub(crate) swapchain: vk::SwapchainKHR,
    pub(crate) format: vk::SurfaceFormatKHR,
    pub(crate) resolution: vk::Extent2D,
    pub(crate) images: Vec<(vk::Image, vk::ImageView)>,
    /// Semaphores for image availability.
    pub(crate) semaphores: Vec<SurfaceImageSemaphores>,
    /// Rolling index for the next available image.
    pub(crate) next_semaphore: usize,
    /// Counter for the number of images acquired.
    pub(crate) images_acquired: usize,
}

pub struct SurfaceImage {
    /// Source surface.
    surface: vk::SurfaceKHR,
    /// Image dimensions.
    dims: (u32, u32),
    /// Actual image object.
    image: vk::Image,
    /// Actual image view object.
    view: vk::ImageView,
    /// Format of the image.
    format: vk::Format,
    /// Index of the surface image acquired.
    image_idx: usize,
    /// Semaphore to wait for availability on.
    semaphores: SurfaceImageSemaphores,
    /// Indicates that the surface image has been used and is available for present.
    used: AtomicBool,
}

#[derive(Copy, Clone)]
pub(crate) struct SurfaceImageSemaphores {
    /// Wait on this semaphore for the image to become available.
    pub available: vk::Semaphore,
    /// Signal this semaphore when the image is ready to be presented.
    pub presentable: vk::Semaphore,
}

impl Surface {
    pub(crate) unsafe fn update_config(
        &mut self,
        ctx: &VulkanBackend,
        config: SurfaceConfiguration,
    ) -> Result<(), SurfaceUpdateError> {
        assert!(config.width != 0, "width was 0");
        assert!(config.height != 0, "height was 0");
        if self.images_acquired != 0 {
            return Err(SurfaceUpdateError::ImagePending);
        }

        self.release(ctx);

        let surface_capabilities = match ctx
            .surface_loader
            .get_physical_device_surface_capabilities(ctx.physical_device, self.surface)
        {
            Ok(capabilities) => capabilities,
            Err(err) => return Err(SurfaceUpdateError::Other(err.to_string())),
        };

        let present_modes = match ctx
            .surface_loader
            .get_physical_device_surface_present_modes(ctx.physical_device, self.surface)
        {
            Ok(present_modes) => present_modes,
            Err(err) => return Err(SurfaceUpdateError::Other(err.to_string())),
        };

        let formats = match ctx
            .surface_loader
            .get_physical_device_surface_formats(ctx.physical_device, self.surface)
        {
            Ok(formats) => formats,
            Err(err) => return Err(SurfaceUpdateError::Other(err.to_string())),
        };

        // Choose number of images
        let mut desired_image_count = surface_capabilities.min_image_count + 1;
        if surface_capabilities.max_image_count > 0
            && desired_image_count > surface_capabilities.max_image_count
        {
            desired_image_count = surface_capabilities.max_image_count;
        }

        // Choose swapchain size based on provided dimensions
        let surface_resolution = vk::Extent2D {
            width: std::cmp::min(config.width, surface_capabilities.max_image_extent.width),
            height: std::cmp::min(config.height, surface_capabilities.max_image_extent.height),
        };

        // No transformation preferred
        let pre_transform = if surface_capabilities
            .supported_transforms
            .contains(vk::SurfaceTransformFlagsKHR::IDENTITY)
        {
            vk::SurfaceTransformFlagsKHR::IDENTITY
        } else {
            surface_capabilities.current_transform
        };

        // Determine a compatible presentation mode and fallback if the requested one is not
        // available.
        let present_mode = {
            let mut present_mode = crate::util::to_vk_present_mode(config.present_mode);

            // Fallback if it's not available
            if !present_modes.contains(&present_mode) {
                present_mode = vk::PresentModeKHR::IMMEDIATE;
            }

            present_mode
        };

        // Determine an approprite format and color space
        self.format = {
            let vk_format = crate::util::to_vk_format(config.format);
            let mut out_format = formats[0];
            for format in formats {
                if format.format == vk_format
                    && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
                {
                    out_format = format;
                }
            }
            out_format
        };

        // Determine if we need exclusive or concurrent access to the images
        let (indices, sharing_mode) = {
            let mut indices = Vec::with_capacity(4);
            let mut sharing_mode = vk::SharingMode::EXCLUSIVE;

            indices.push(ctx.queue_family_indices.present);

            if ctx.queue_family_indices.present != ctx.queue_family_indices.main {
                indices.push(ctx.queue_family_indices.main);
                sharing_mode = vk::SharingMode::CONCURRENT;
            }

            if ctx.queue_family_indices.present != ctx.queue_family_indices.transfer {
                indices.push(ctx.queue_family_indices.transfer);
                sharing_mode = vk::SharingMode::CONCURRENT;
            }

            (indices, sharing_mode)
        };

        // Create the swapchain
        let swapchain_create_info = vk::SwapchainCreateInfoKHR::builder()
            .surface(self.surface)
            .min_image_count(desired_image_count)
            .image_color_space(self.format.color_space)
            .image_format(self.format.format)
            .image_extent(surface_resolution)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST)
            .image_sharing_mode(sharing_mode)
            .queue_family_indices(&indices)
            .pre_transform(pre_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true)
            .image_array_layers(1);

        self.swapchain = match ctx
            .swapchain_loader
            .create_swapchain(&swapchain_create_info, None)
        {
            Ok(swapchain) => swapchain,
            Err(err) => return Err(SurfaceUpdateError::Other(err.to_string())),
        };

        // Get swapchain images
        self.images = match ctx.swapchain_loader.get_swapchain_images(self.swapchain) {
            Ok(images) => images
                .into_iter()
                .map(|image| {
                    let create_info = vk::ImageViewCreateInfo::builder()
                        .image(image)
                        .components(vk::ComponentMapping {
                            r: vk::ComponentSwizzle::R,
                            g: vk::ComponentSwizzle::G,
                            b: vk::ComponentSwizzle::B,
                            a: vk::ComponentSwizzle::A,
                        })
                        .subresource_range(vk::ImageSubresourceRange {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            base_mip_level: 0,
                            level_count: 1,
                            base_array_layer: 0,
                            layer_count: 1,
                        })
                        .format(self.format.format)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .build();

                    let view = ctx.device.create_image_view(&create_info, None).unwrap();

                    (image, view)
                })
                .collect(),
            Err(err) => return Err(SurfaceUpdateError::Other(err.to_string())),
        };

        // Create image availability semaphores
        for _ in 0..self.images.len() {
            let create_info = vk::SemaphoreCreateInfo::default();
            let available = ctx.device.create_semaphore(&create_info, None).unwrap();
            let presentable = ctx.device.create_semaphore(&create_info, None).unwrap();
            self.semaphores.push(SurfaceImageSemaphores {
                available,
                presentable,
            });
        }

        self.next_semaphore = 0;
        self.resolution = vk::Extent2D {
            width: surface_resolution.width,
            height: surface_resolution.height,
        };

        Ok(())
    }

    #[inline]
    pub(crate) fn dimensions(&self) -> (u32, u32) {
        (self.resolution.width, self.resolution.height)
    }

    pub(crate) unsafe fn acquire_image(
        &mut self,
        ctx: &VulkanBackend,
    ) -> Result<SurfaceImage, SurfaceImageAcquireError> {
        if self.images_acquired + 1 > self.images.len() {
            return Err(SurfaceImageAcquireError::NoImages);
        }

        // Acquire the image
        self.next_semaphore = (self.next_semaphore + 1) % self.semaphores.len();
        let semaphores = self.semaphores[self.next_semaphore];
        let image_idx = match ctx.swapchain_loader.acquire_next_image(
            self.swapchain,
            u64::MAX,
            semaphores.available,
            vk::Fence::null(),
        ) {
            Ok((idx, _)) => idx as usize,
            Err(err) => return Err(SurfaceImageAcquireError::Other(err.to_string())),
        };

        Ok(SurfaceImage {
            surface: self.surface,
            dims: (self.resolution.width, self.resolution.height),
            image: self.images[image_idx].0,
            view: self.images[image_idx].1,
            format: self.format.format,
            image_idx,
            semaphores,
            used: AtomicBool::new(false),
        })
    }

    pub(crate) unsafe fn release(&mut self, ctx: &VulkanBackend) {
        for semaphores in self.semaphores.drain(..) {
            ctx.device.destroy_semaphore(semaphores.available, None);
            ctx.device.destroy_semaphore(semaphores.presentable, None);
        }

        for (_, view) in self.images.drain(..) {
            ctx.device.destroy_image_view(view, None);
        }

        if self.swapchain != vk::SwapchainKHR::null() {
            ctx.swapchain_loader.destroy_swapchain(self.swapchain, None);
        }
    }
}

impl SurfaceImage {
    #[inline(always)]
    pub(crate) fn image(&self) -> vk::Image {
        self.image
    }

    #[inline(always)]
    pub(crate) fn index(&self) -> usize {
        self.image_idx
    }

    #[inline(always)]
    pub(crate) fn view(&self) -> vk::ImageView {
        self.view
    }

    #[inline(always)]
    pub(crate) fn surface(&self) -> vk::SurfaceKHR {
        self.surface
    }

    #[inline(always)]
    pub(crate) fn is_signaled(&self) -> bool {
        self.used.load(Ordering::Relaxed)
    }

    #[inline(always)]
    pub(crate) fn signal_draw(&self) {
        self.used.store(true, Ordering::Relaxed);
    }

    #[inline(always)]
    pub(crate) fn semaphores(&self) -> &SurfaceImageSemaphores {
        &self.semaphores
    }

    #[inline(always)]
    pub(crate) fn dims(&self) -> (u32, u32) {
        self.dims
    }

    #[inline(always)]
    pub(crate) fn format(&self) -> vk::Format {
        self.format
    }
}
