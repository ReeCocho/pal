use api::{
    buffer::Buffer,
    command_buffer::{BufferTextureCopy, Command, CopyBufferToBuffer},
    descriptor_set::DescriptorSet,
    render_pass::{ColorAttachmentSource, RenderPassDescriptor},
    texture::Texture,
};

use super::{
    semaphores::{SemaphoreTracker, WaitInfo},
    usage::{PipelineTracker, SubResource, SubResourceUsage, UsageScope},
};
use crate::descriptor_set::BoundValue;
use ash::vk;

pub(crate) struct TrackState<'a, 'b> {
    pub device: &'a ash::Device,
    pub command_buffer: vk::CommandBuffer,
    /// Index of the command to detect the resources of.
    pub index: usize,
    /// Command list with all commands of a submit.
    pub commands: &'a [Command<'a, crate::VulkanBackend>],
    /// Used to detect inter-command dependencies.
    pub pipeline_tracker: &'a mut PipelineTracker<'b>,
    /// Used by `resc_state` to track inter-queue dependencies.
    pub semaphores: &'a mut SemaphoreTracker,
}

/// Given the index of a command in a command list, tracks resources based off the type of
/// detected command.
pub(crate) unsafe fn track_resources(mut state: TrackState) {
    match &state.commands[state.index] {
        Command::BeginRenderPass(descriptor) => track_render_pass(&mut state, descriptor),
        Command::Dispatch(_, _, _) => track_dispatch(&mut state),
        Command::CopyBufferToBuffer(copy_info) => {
            track_buffer_to_buffer_copy(&mut state, copy_info)
        }
        Command::CopyBufferToTexture {
            buffer,
            texture,
            copy,
        } => track_buffer_to_texture_copy(&mut state, buffer, texture, copy),
        Command::CopyTextureToBuffer {
            buffer,
            texture,
            copy,
        } => track_texture_to_buffer_copy(&mut state, buffer, texture, copy),
        // All other commands do not need state tracking
        _ => {}
    }
}

unsafe fn track_render_pass(
    state: &mut TrackState,
    descriptor: &RenderPassDescriptor<'_, crate::VulkanBackend>,
) {
    let mut scope = UsageScope::default();

    // Track color attachments used in the pass
    for attachment in &descriptor.color_attachments {
        let subresource = match attachment.source {
            ColorAttachmentSource::SurfaceImage(image) => {
                // Surface image has special semaphores
                let semaphores = image.internal().semaphores();
                state
                    .semaphores
                    .register_signal(semaphores.presentable, None);
                state.semaphores.register_wait(
                    semaphores.available,
                    WaitInfo {
                        value: None,
                        stage: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                    },
                );

                SubResource::Texture {
                    texture: image.internal().image(),
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    array_elem: 0,
                    mip_level: 0,
                }
            }
            ColorAttachmentSource::Texture {
                texture,
                array_element,
                mip_level,
            } => SubResource::Texture {
                texture: texture.internal().image,
                aspect_mask: vk::ImageAspectFlags::COLOR,
                array_elem: array_element as u32,
                mip_level: mip_level as u32,
            },
        };
        scope.use_resource(
            subresource,
            SubResourceUsage {
                access: vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::COLOR_ATTACHMENT_READ,
                stage: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            },
        );
    }

    // Track depth stencil attachment
    if let Some(attachment) = &descriptor.depth_stencil_attachment {
        let internal = attachment.texture.internal();
        scope.use_resource(
            SubResource::Texture {
                texture: internal.image,
                aspect_mask: internal.aspect_flags,
                array_elem: attachment.array_element as u32,
                mip_level: attachment.mip_level as u32,
            },
            SubResourceUsage {
                access: vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                stage: vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                    | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
            },
        );
    }

    // Track everything else
    for command in &state.commands[state.index..] {
        match command {
            Command::BindVertexBuffers { binds, .. } => {
                for bind in binds {
                    scope.use_resource(
                        SubResource::Buffer {
                            buffer: bind.buffer.internal().buffer,
                            array_elem: bind.array_element as u32,
                        },
                        SubResourceUsage {
                            access: vk::AccessFlags::VERTEX_ATTRIBUTE_READ,
                            stage: vk::PipelineStageFlags::VERTEX_INPUT,
                            layout: vk::ImageLayout::UNDEFINED,
                        },
                    );
                }
            }
            Command::BindIndexBuffer {
                buffer,
                array_element,
                ..
            } => {
                scope.use_resource(
                    SubResource::Buffer {
                        buffer: buffer.internal().buffer,
                        array_elem: *array_element as u32,
                    },
                    SubResourceUsage {
                        access: vk::AccessFlags::INDEX_READ,
                        stage: vk::PipelineStageFlags::VERTEX_INPUT,
                        layout: vk::ImageLayout::UNDEFINED,
                    },
                );
            }
            Command::BindDescriptorSets { sets, .. } => {
                track_descriptor_sets(sets, &mut scope);
            }
            Command::EndRenderPass => break,
            _ => {}
        }
    }

    // Submit usage scope
    if let Some(barrier) = state.pipeline_tracker.submit(scope) {
        barrier.execute(state.device, state.command_buffer);
    }
}

unsafe fn track_dispatch(state: &mut TrackState) {
    // Find the index of the bound pipeline
    let idx = {
        let mut idx = None;
        for (i, command) in state.commands[..=state.index].iter().enumerate().rev() {
            match command {
                Command::BindComputePipeline(_) => {
                    idx = Some(i);
                    break;
                }
                Command::BeginComputePass => break,
                _ => {}
            }
        }

        match idx {
            Some(idx) => idx,
            // No bound pipeline so no state track needed
            None => return,
        }
    };

    // Determine how many sets are used by the active pipeline
    let mut total_bound = 0;
    let mut bound = {
        let pipeline = match &state.commands[idx] {
            Command::BindComputePipeline(pipeline) => pipeline,
            // Unreachable because of early return in previous pass
            _ => unreachable!(),
        };
        let mut bound = Vec::with_capacity(pipeline.layouts().len());
        bound.resize(pipeline.layouts().len(), false);
        bound
    };

    let mut scope = UsageScope::default();

    // Determine which sets are actually used
    for command in state.commands[idx..=state.index].iter().rev() {
        // Break early if every set is bound
        if total_bound == bound.len() {
            break;
        }

        // Grab bind info. Skip other commands
        let (sets, first) = match command {
            Command::BindDescriptorSets { sets, first, .. } => (sets, *first),
            _ => continue,
        };

        // Track sets
        for (i, set_slot) in (first..(first + sets.len())).into_iter().enumerate() {
            // Skip if the set slot is already bound
            if bound[set_slot] {
                continue;
            }

            // Track
            track_descriptor_set(sets[i], &mut scope);
            bound[set_slot] = true;
            total_bound += 1;
        }
    }

    // Submit pipeline values
    if let Some(barrier) = state.pipeline_tracker.submit(scope) {
        barrier.execute(state.device, state.command_buffer);
    }
}

unsafe fn track_buffer_to_buffer_copy(
    state: &mut TrackState,
    copy: &CopyBufferToBuffer<'_, crate::VulkanBackend>,
) {
    // Barrier check
    let src = copy.src.internal();
    let dst = copy.dst.internal();
    let mut scope = UsageScope::default();
    scope.use_resource(
        SubResource::Buffer {
            buffer: src.buffer,
            array_elem: copy.src_array_element as u32,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_READ,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::UNDEFINED,
        },
    );
    scope.use_resource(
        SubResource::Buffer {
            buffer: dst.buffer,
            array_elem: copy.dst_array_element as u32,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_WRITE,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::UNDEFINED,
        },
    );

    if let Some(barrier) = state.pipeline_tracker.submit(scope) {
        barrier.execute(state.device, state.command_buffer);
    }
}

unsafe fn track_buffer_to_texture_copy(
    state: &mut TrackState,
    buffer: &Buffer<crate::VulkanBackend>,
    texture: &Texture<crate::VulkanBackend>,
    copy: &BufferTextureCopy,
) {
    // Barrier check
    let buffer = buffer.internal();
    let texture = texture.internal();
    let mut scope = UsageScope::default();
    scope.use_resource(
        SubResource::Buffer {
            buffer: buffer.buffer,
            array_elem: copy.buffer_array_element as u32,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_READ,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::UNDEFINED,
        },
    );
    scope.use_resource(
        SubResource::Texture {
            texture: texture.image,
            aspect_mask: texture.aspect_flags,
            array_elem: copy.texture_array_element as u32,
            mip_level: copy.texture_mip_level as u32,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_WRITE,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        },
    );

    if let Some(barrier) = state.pipeline_tracker.submit(scope) {
        barrier.execute(state.device, state.command_buffer);
    }
}

unsafe fn track_texture_to_buffer_copy(
    state: &mut TrackState,
    buffer: &Buffer<crate::VulkanBackend>,
    texture: &Texture<crate::VulkanBackend>,
    copy: &BufferTextureCopy,
) {
    // Barrier check
    let buffer = buffer.internal();
    let texture = texture.internal();
    let mut scope = UsageScope::default();
    scope.use_resource(
        SubResource::Buffer {
            buffer: buffer.buffer,
            array_elem: copy.buffer_array_element as u32,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_WRITE,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::UNDEFINED,
        },
    );
    scope.use_resource(
        SubResource::Texture {
            texture: texture.image,
            aspect_mask: texture.aspect_flags,
            array_elem: copy.texture_array_element as u32,
            mip_level: copy.texture_mip_level as u32,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_READ,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
        },
    );

    if let Some(barrier) = state.pipeline_tracker.submit(scope) {
        barrier.execute(state.device, state.command_buffer);
    }
}

unsafe fn track_descriptor_sets(
    sets: &[&DescriptorSet<crate::VulkanBackend>],
    scope: &mut UsageScope,
) {
    for set in sets.into_iter() {
        track_descriptor_set(set, scope);
    }
}

unsafe fn track_descriptor_set(set: &DescriptorSet<crate::VulkanBackend>, scope: &mut UsageScope) {
    // Check every binding of every set
    for binding in &set.internal().bound {
        // Check every element of every binding
        for elem in binding {
            // Only care about elements if they are filled
            if let Some(elem) = elem {
                match &elem.value {
                    BoundValue::UniformBuffer {
                        buffer,
                        array_element,
                        ..
                    } => scope.use_resource(
                        SubResource::Buffer {
                            buffer: *buffer,
                            array_elem: *array_element as u32,
                        },
                        SubResourceUsage {
                            access: elem.access,
                            stage: elem.stage,
                            layout: vk::ImageLayout::UNDEFINED,
                        },
                    ),
                    BoundValue::StorageBuffer {
                        buffer,
                        array_element,
                        ..
                    } => scope.use_resource(
                        SubResource::Buffer {
                            buffer: *buffer,
                            array_elem: *array_element as u32,
                        },
                        SubResourceUsage {
                            access: elem.access,
                            stage: elem.stage,
                            layout: vk::ImageLayout::UNDEFINED,
                        },
                    ),
                    // Textures require that you register each mip individually
                    BoundValue::Texture {
                        _ref_counter,
                        image,
                        array_element,
                        aspect_mask,
                        mip_count,
                        ..
                    } => {
                        for i in 0..*mip_count {
                            scope.use_resource(
                                SubResource::Texture {
                                    texture: *image,
                                    aspect_mask: *aspect_mask,
                                    array_elem: *array_element as u32,
                                    mip_level: i,
                                },
                                SubResourceUsage {
                                    access: elem.access,
                                    stage: elem.stage,
                                    layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                                },
                            )
                        }
                    }
                }
            }
        }
    }
}
