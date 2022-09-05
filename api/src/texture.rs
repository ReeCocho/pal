use crate::{context::Context, Backend};

pub struct TextureCreateInfo<B: Backend> {
    pub ctx: Context<B>,
}
