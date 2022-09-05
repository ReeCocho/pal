use std::collections::{HashMap, HashSet};

use api::{
    render_pass::{ColorAttachmentSource, RenderPassDescriptor},
    types::LoadOp,
};
use ash::vk;
use dashmap::DashMap;

#[derive(Default)]
pub(crate) struct RenderPassCache {
    passes: DashMap<VkRenderPassDescriptor, vk::RenderPass>,
}

#[derive(Default)]
pub(crate) struct FramebufferCache {
    /// Maps a render pass to their framebuffers.
    pass_to_framebuffers: DashMap<vk::RenderPass, Framebuffers>,
    /// Maps images to what render passes have framebuffers containing them.
    image_to_pass: DashMap<vk::ImageView, HashSet<vk::RenderPass>>,
}

#[derive(Default)]
pub(crate) struct Framebuffers {
    pass: vk::RenderPass,
    /// Maps an ordered set of images to a framebuffer.
    framebuffers: HashMap<Vec<vk::ImageView>, vk::Framebuffer>,
}

#[derive(Default, Hash, PartialEq, Eq)]
pub(crate) struct VkRenderPassDescriptor {
    pub color_attachments: Vec<VkAttachment>,
    pub depth_stencil_attachment: Option<VkAttachment>,
}

#[derive(Hash, PartialEq, Eq)]
pub(crate) struct VkAttachment {
    pub image_format: vk::Format,
    pub load_op: vk::AttachmentLoadOp,
    pub store_op: vk::AttachmentStoreOp,
}

impl RenderPassCache {
    /// Checks if a compatible render pass is in the cache. If it is, it is returned. Otherwise,
    /// a new render pass is created and returned.
    pub fn get(
        &self,
        device: &ash::Device,
        pass: &RenderPassDescriptor<crate::VulkanBackend>,
    ) -> vk::RenderPass {
        let descriptor = VkRenderPassDescriptor::from_descriptor(pass);
        *self.passes.entry(descriptor).or_insert_with(|| {
            // Create attachment descriptors
            let mut attachments = Vec::with_capacity(pass.color_attachments.len());
            for attachment in &pass.color_attachments {
                attachments.push(
                    vk::AttachmentDescription::builder()
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .initial_layout(match attachment.load_op {
                            LoadOp::Load => vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                            LoadOp::DontCare => vk::ImageLayout::UNDEFINED,
                            LoadOp::Clear(_) => vk::ImageLayout::UNDEFINED,
                        })
                        .final_layout(match &attachment.source {
                            ColorAttachmentSource::SurfaceImage(_) => {
                                vk::ImageLayout::PRESENT_SRC_KHR
                            }
                        })
                        .load_op(crate::util::to_vk_load_op(attachment.load_op))
                        .store_op(crate::util::to_vk_store_op(attachment.store_op))
                        .format(match &attachment.source {
                            ColorAttachmentSource::SurfaceImage(image) => image.internal().format(),
                        })
                        .build(),
                );
            }

            // Link with attachment references
            let mut attachment_refs = Vec::with_capacity(pass.color_attachments.len());
            for (i, _) in pass.color_attachments.iter().enumerate() {
                attachment_refs.push(
                    vk::AttachmentReference::builder()
                        .attachment(i as u32)
                        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                        .build(),
                );
            }

            // Single subpass
            let subpass = [vk::SubpassDescription::builder()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&attachment_refs)
                .build()];

            // Create the render pass
            unsafe {
                let create_info = vk::RenderPassCreateInfo::builder()
                    .attachments(&attachments)
                    .subpasses(&subpass)
                    .build();

                device.create_render_pass(&create_info, None).unwrap()
            }
        })
    }

    pub unsafe fn release(&self, device: &ash::Device) {
        for pass in self.passes.iter() {
            device.destroy_render_pass(*pass.value(), None);
        }
    }
}

impl FramebufferCache {
    /// Given an ordered set of images and a render pass, produces a framebuffer.
    pub fn get(
        &self,
        device: &ash::Device,
        render_pass: vk::RenderPass,
        images: Vec<vk::ImageView>,
        extent: vk::Extent2D,
    ) -> vk::Framebuffer {
        // Get the framebuffers of the pass
        let mut framebuffers =
            self.pass_to_framebuffers
                .entry(render_pass)
                .or_insert(Framebuffers {
                    pass: render_pass,
                    framebuffers: HashMap::default(),
                });

        // Associate images to pass
        for image in &images {
            let mut passes = self.image_to_pass.entry(*image).or_default();
            passes.insert(render_pass);
        }

        // Get the framebuffer
        framebuffers.get(device, images, extent)
    }

    /// Call when an image is destroyed so framebuffers can be cleaned up.
    pub unsafe fn view_destroyed(&self, device: &ash::Device, image: vk::ImageView) {
        // Get the associated passes
        let passes = match self.image_to_pass.get(&image) {
            Some(passes) => passes,
            None => return,
        };

        // Loop over every pass and signal each one that the view is destroyed
        for pass in passes.value() {
            let mut framebuffers = match self.pass_to_framebuffers.get_mut(pass) {
                Some(framebuffers) => framebuffers,
                None => continue,
            };
            framebuffers.view_destroyed(device, image);
        }
    }

    pub unsafe fn release(&self, device: &ash::Device) {
        for entry in self.pass_to_framebuffers.iter() {
            entry.release(device);
        }
    }
}

impl Framebuffers {
    #[inline(always)]
    pub fn get(
        &mut self,
        device: &ash::Device,
        images: Vec<vk::ImageView>,
        extent: vk::Extent2D,
    ) -> vk::Framebuffer {
        match self.framebuffers.get(&images) {
            Some(framebuffer) => *framebuffer,
            None => {
                let framebuffer = unsafe {
                    let create_info = vk::FramebufferCreateInfo::builder()
                        .attachments(&images)
                        .width(extent.width)
                        .height(extent.height)
                        .layers(1)
                        .render_pass(self.pass)
                        .build();

                    device.create_framebuffer(&create_info, None).unwrap()
                };
                self.framebuffers.insert(images, framebuffer);
                framebuffer
            }
        }
    }

    /// Call when an image is destroyed so framebuffers can be cleaned up.
    pub unsafe fn view_destroyed(&mut self, device: &ash::Device, image: vk::ImageView) {
        // TODO: Could probably make this faster
        let mut to_remove = Vec::default();
        for (key, value) in &self.framebuffers {
            if key.contains(&image) {
                // Destroy the framebuffer
                device.destroy_framebuffer(*value, None);

                // Signal to remove
                to_remove.push(key.clone());
            }
        }

        // Remove indicated
        for to_remove in to_remove {
            self.framebuffers.remove(&to_remove);
        }
    }

    pub unsafe fn release(&self, device: &ash::Device) {
        for framebuffer in self.framebuffers.values() {
            device.destroy_framebuffer(*framebuffer, None);
        }
    }
}

impl VkRenderPassDescriptor {
    pub fn from_descriptor<'a>(
        descriptor: &RenderPassDescriptor<'a, crate::VulkanBackend>,
    ) -> VkRenderPassDescriptor {
        let mut out = VkRenderPassDescriptor::default();
        for attachment in &descriptor.color_attachments {
            out.color_attachments.push(VkAttachment {
                image_format: match &attachment.source {
                    ColorAttachmentSource::SurfaceImage(image) => image.internal().format(),
                },
                load_op: crate::util::to_vk_load_op(attachment.load_op),
                store_op: crate::util::to_vk_store_op(attachment.store_op),
            });
        }
        out
    }
}
