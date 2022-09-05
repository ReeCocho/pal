use std::sync::Arc;

use crate::{queue::Queue, types::QueueType, Backend};

pub struct Context<B: Backend>(pub(crate) Arc<B>);

impl<B: Backend> Context<B> {
    #[inline(always)]
    pub fn new(backend: B) -> Self {
        Self(Arc::new(backend))
    }

    #[inline(always)]
    pub fn main(&self) -> Queue<B> {
        Queue::new(self.clone(), QueueType::Main)
    }

    #[inline(always)]
    pub fn transfer(&self) -> Queue<B> {
        Queue::new(self.clone(), QueueType::Transfer)
    }

    #[inline(always)]
    pub fn compute(&self) -> Queue<B> {
        Queue::new(self.clone(), QueueType::Compute)
    }

    #[inline(always)]
    pub fn present(&self) -> Queue<B> {
        Queue::new(self.clone(), QueueType::Present)
    }
}

impl<B: Backend> Clone for Context<B> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
