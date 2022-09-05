use ash::vk;

pub struct Shader {
    pub(crate) module: vk::ShaderModule,
}
