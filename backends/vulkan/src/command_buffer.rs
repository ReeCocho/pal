/*
use api::{command_buffer::CommandBufferSubmitError, render_pass::RenderPass};
use ash::vk;

use crate::{queue::QueueInnerRef, render_pass::RenderPassCommands};

pub struct CommandBuffer {
    queue: QueueInnerRef,
}

impl api::command_buffer::CommandBuffer<crate::Context> for CommandBuffer {
    fn render_pass<'a>(
        &mut self,
        render_pass: RenderPass<'a, crate::Context>,
        func: impl FnOnce(&mut RenderPassCommands),
    ) {
        todo!()
    }

    fn submit(self) -> Result<(), api::command_buffer::CommandBufferSubmitError> {
        let mut queue = self.queue.write().unwrap();
        let ctx = queue.ctx.upgrade().unwrap();

        // Allocate and begin the command buffer
        let command_buffer = unsafe {
            let command_buffer = queue.allocate_command_buffer(&ctx.device);
            let begin_info = vk::CommandBufferBeginInfo::builder()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
                .build();
            if let Err(err) = ctx.device.begin_command_buffer(command_buffer, &begin_info) {
                return Err(CommandBufferSubmitError::Other(err.to_string()));
            }
            command_buffer
        };

        // TODO: Decode commands

        // Submit the command buffer to the queue
        unsafe {
            if let Err(err) = ctx.device.end_command_buffer(command_buffer) {
                return Err(CommandBufferSubmitError::Other(err.to_string()));
            }
            if let Err(err) = queue.submit(&ctx.device, command_buffer) {
                return Err(CommandBufferSubmitError::Other(err.to_string()));
            }
        }

        Ok(())
    }
}
*/
