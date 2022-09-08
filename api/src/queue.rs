use std::time::Duration;

use crate::{
    command_buffer::CommandBuffer,
    context::Context,
    surface::{Surface, SurfaceImage, SurfacePresentError, SurfacePresentSuccess},
    types::{JobStatus, QueueType},
    Backend,
};

pub struct Queue<B: Backend> {
    ctx: Context<B>,
    ty: QueueType,
}

pub struct Job<B: Backend> {
    ctx: Context<B>,
    id: B::Job,
}

pub enum SurfacePresentFailure {
    BadImage,
    NoRender,
    Other(String),
}

impl<B: Backend> Queue<B> {
    pub(crate) fn new(ctx: Context<B>, ty: QueueType) -> Self {
        Self { ctx, ty }
    }

    #[inline(always)]
    pub fn ty(&self) -> QueueType {
        self.ty
    }

    #[inline(always)]
    pub fn submit<'a>(
        &self,
        debug_name: Option<&str>,
        commands: impl FnOnce(&mut CommandBuffer<'a, B>),
    ) -> Job<B> {
        let mut cb = CommandBuffer {
            commands: Vec::default(),
        };
        commands(&mut cb);
        let id = unsafe { self.ctx.0.submit_commands(self.ty, debug_name, cb.commands) };

        Job {
            id,
            ctx: self.ctx.clone(),
        }
    }

    #[inline(always)]
    pub fn present(
        &self,
        surface: &Surface<B>,
        mut image: SurfaceImage<B>,
    ) -> Result<SurfacePresentSuccess, SurfacePresentError<B>> {
        unsafe {
            match self.ctx.0.present_image(&surface.id, &mut image.id) {
                Ok(success) => Ok(success),
                Err(err) => match err {
                    SurfacePresentFailure::BadImage => Err(SurfacePresentError::BadImage(image)),
                    SurfacePresentFailure::NoRender => Err(SurfacePresentError::NoRender(image)),
                    SurfacePresentFailure::Other(msg) => Err(SurfacePresentError::Other(msg)),
                },
            }
        }
    }
}

impl<B: Backend> Job<B> {
    /// Wait's for the job to complete with the given timeout. If `None` is provided, then this
    /// call will block as long as possible for the job is finished. Returns the status of the
    /// job by the time the timeout is reached.
    #[inline(always)]
    fn wait_on(&self, timeout: Option<Duration>) -> JobStatus {
        unsafe { self.ctx.0.wait_on(&self.id, timeout) }
    }

    /// Polls the current status of the job without blocking.
    #[inline(always)]
    fn poll_status(&self) -> JobStatus {
        unsafe { self.ctx.0.poll_status(&self.id) }
    }
}
