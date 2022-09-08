use api::{
    command_buffer::{Command, CopyBufferToBuffer},
    descriptor_set::DescriptorSet,
    render_pass::{ColorAttachmentSource, RenderPassDescriptor},
    types::QueueType,
};

use super::{
    pipeline_tracker::{PipelineTracker, ResourceUsage},
    resource_state::{LatestUsage, ResourceState},
    semaphores::{SemaphoreTracker, WaitInfo},
};
use crate::{descriptor_set::BoundValue, queue::VkQueue};
use ash::vk;

pub(crate) struct TrackState<'a> {
    pub device: &'a ash::Device,
    pub command_buffer: vk::CommandBuffer,
    /// Index of the command to detect the resources of.
    pub index: usize,
    /// Command list with all commands of a submit.
    pub commands: &'a [Command<'a, crate::VulkanBackend>],
    /// Used to detect inter-command dependencies.
    pub pipeline_tracker: &'a mut PipelineTracker,
    /// Used to detect inter-queue dependencies.
    pub resc_state: &'a mut ResourceState,
    /// Used by `resc_state` to track inter-queue dependencies.
    pub semaphores: &'a mut SemaphoreTracker,
    /// Queue type being used.
    pub queue_ty: QueueType,
    /// Target value of the queue being used (after these commands are submitted.)
    pub target_value: u64,
    // Queues
    pub main: &'a VkQueue,
    pub transfer: &'a VkQueue,
    pub compute: &'a VkQueue,
    pub present: &'a VkQueue,
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
        // All other commands do not need state tracking
        _ => {}
    }
}

unsafe fn track_render_pass(
    state: &mut TrackState,
    descriptor: &RenderPassDescriptor<'_, crate::VulkanBackend>,
) {
    let mut buffers_pipeline = Vec::default();
    let mut images_pipeline = Vec::with_capacity(descriptor.color_attachments.len());
    let mut buffers_semaphore = Vec::default();
    // TODO: let mut images_semaphore = Vec::with_capacity(descriptor.color_attachments.len());

    // Track images used in the pass
    for attachment in &descriptor.color_attachments {
        let image = match attachment.source {
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

                image.internal().image()
            }
        };
        images_pipeline.push((
            image,
            ResourceUsage {
                used: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                access: vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::COLOR_ATTACHMENT_READ,
            },
        ));
    }

    // Track everything else
    for command in &state.commands[state.index..] {
        match command {
            Command::BindVertexBuffers { binds, .. } => {
                for bind in binds {
                    buffers_pipeline.push((
                        bind.buffer.internal().buffer,
                        ResourceUsage {
                            used: vk::PipelineStageFlags::VERTEX_INPUT,
                            access: vk::AccessFlags::VERTEX_ATTRIBUTE_READ,
                        },
                    ));
                    buffers_semaphore.push((
                        bind.buffer.internal().buffer,
                        bind.array_element,
                        vk::PipelineStageFlags::VERTEX_INPUT,
                    ));
                }
            }
            Command::BindIndexBuffer {
                buffer,
                array_element,
                ..
            } => {
                buffers_pipeline.push((
                    buffer.internal().buffer,
                    ResourceUsage {
                        used: vk::PipelineStageFlags::VERTEX_INPUT,
                        access: vk::AccessFlags::INDEX_READ,
                    },
                ));
                buffers_semaphore.push((
                    buffer.internal().buffer,
                    *array_element,
                    vk::PipelineStageFlags::VERTEX_INPUT,
                ));
            }
            Command::BindDescriptorSets { sets, .. } => {
                track_descriptor_sets(sets, &mut buffers_pipeline, &mut buffers_semaphore);
            }
            Command::EndRenderPass => break,
            _ => {}
        }
    }

    // Submit pipeline values
    if let Some(barrier) = state
        .pipeline_tracker
        .update(&buffers_pipeline, &images_pipeline)
    {
        barrier.execute(&state.device, state.command_buffer);
    }

    // Submit semaphore values
    for (buffer, array_elem, stage) in buffers_semaphore {
        if let Some(old) = state.resc_state.set_buffer(
            buffer,
            array_elem,
            Some(LatestUsage {
                queue: state.queue_ty,
                value: state.target_value,
            }),
        ) {
            old.wait_if_needed(
                state.semaphores,
                state.queue_ty,
                stage,
                state.main,
                state.transfer,
                state.compute,
                state.present,
            );
        }
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

    // Determine which sets are actually used
    let mut buffers_pipeline = Vec::default();
    // TODO: let mut images_pipeline = Vec::with_capacity();
    let mut buffers_semaphore = Vec::default();
    // TODO: let mut images_semaphore = Vec::with_capacity();

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
            track_descriptor_set(sets[i], &mut buffers_pipeline, &mut buffers_semaphore);
            bound[set_slot] = true;
            total_bound += 1;
        }
    }

    // Submit pipeline values
    if let Some(barrier) = state.pipeline_tracker.update(&buffers_pipeline, &[]) {
        barrier.execute(&state.device, state.command_buffer);
    }

    // Submit semaphore values
    for (buffer, array_elem, stage) in buffers_semaphore {
        if let Some(old) = state.resc_state.set_buffer(
            buffer,
            array_elem,
            Some(LatestUsage {
                queue: state.queue_ty,
                value: state.target_value,
            }),
        ) {
            old.wait_if_needed(
                state.semaphores,
                state.queue_ty,
                stage,
                state.main,
                state.transfer,
                state.compute,
                state.present,
            );
        }
    }
}

unsafe fn track_buffer_to_buffer_copy(
    state: &mut TrackState,
    copy: &CopyBufferToBuffer<'_, crate::VulkanBackend>,
) {
    // Barrier check
    let src = copy.src.internal();
    let dst = copy.dst.internal();
    if let Some(barrier) = state.pipeline_tracker.update(
        &[
            (
                src.buffer,
                ResourceUsage {
                    used: vk::PipelineStageFlags::TRANSFER,
                    access: vk::AccessFlags::TRANSFER_READ,
                },
            ),
            (
                dst.buffer,
                ResourceUsage {
                    used: vk::PipelineStageFlags::TRANSFER,
                    access: vk::AccessFlags::TRANSFER_WRITE,
                },
            ),
        ],
        &[],
    ) {
        barrier.execute(&state.device, state.command_buffer);
    }

    // Semaphore check
    if let Some(old) = state.resc_state.set_buffer(
        src.buffer,
        copy.src_array_element,
        Some(LatestUsage {
            queue: state.queue_ty,
            value: state.target_value,
        }),
    ) {
        old.wait_if_needed(
            state.semaphores,
            state.queue_ty,
            vk::PipelineStageFlags::TRANSFER,
            state.main,
            state.transfer,
            state.compute,
            state.present,
        );
    }

    if let Some(old) = state.resc_state.set_buffer(
        dst.buffer,
        copy.dst_array_element,
        Some(LatestUsage {
            queue: state.queue_ty,
            value: state.target_value,
        }),
    ) {
        old.wait_if_needed(
            state.semaphores,
            state.queue_ty,
            vk::PipelineStageFlags::TRANSFER,
            state.main,
            state.transfer,
            state.compute,
            state.present,
        );
    }
}

unsafe fn track_descriptor_sets(
    sets: &[&DescriptorSet<crate::VulkanBackend>],
    buffers_pipeline: &mut Vec<(vk::Buffer, ResourceUsage)>,
    buffers_semaphores: &mut Vec<(vk::Buffer, usize, vk::PipelineStageFlags)>,
) {
    for set in sets.into_iter() {
        track_descriptor_set(set, buffers_pipeline, buffers_semaphores);
    }
}

unsafe fn track_descriptor_set(
    set: &DescriptorSet<crate::VulkanBackend>,
    buffers_pipeline: &mut Vec<(vk::Buffer, ResourceUsage)>,
    buffers_semaphores: &mut Vec<(vk::Buffer, usize, vk::PipelineStageFlags)>,
) {
    // Check every binding of every set
    for binding in &set.internal().bound {
        // Check every element of every binding
        for elem in binding {
            // Only care about elements if they are filled
            if let Some(elem) = elem {
                let usage = ResourceUsage {
                    used: elem.stage,
                    access: elem.access,
                };
                match &elem.value {
                    BoundValue::UniformBuffer {
                        buffer,
                        array_element,
                        ..
                    } => {
                        buffers_pipeline.push((*buffer, usage));
                        buffers_semaphores.push((*buffer, *array_element, elem.stage));
                    }
                    BoundValue::StorageBuffer {
                        buffer,
                        array_element,
                        ..
                    } => {
                        buffers_pipeline.push((*buffer, usage));
                        buffers_semaphores.push((*buffer, *array_element, elem.stage));
                    }
                }
            }
        }
    }
}
