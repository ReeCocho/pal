use std::ffi::CString;

use api::shader::{ShaderCreateError, ShaderCreateInfo};
use ash::vk::{self, Handle};

pub struct Shader {
    pub(crate) module: vk::ShaderModule,
}

impl Shader {
    pub(crate) unsafe fn new(
        device: &ash::Device,
        debug: Option<&ash::extensions::ext::DebugUtils>,
        create_info: ShaderCreateInfo,
    ) -> Result<Self, ShaderCreateError> {
        let code = match bytemuck::try_cast_slice::<u8, u32>(create_info.code) {
            Ok(code) => code,
            Err(_) => {
                return Err(ShaderCreateError::Other(String::from(
                    "shader code size is not a multiple of 4",
                )))
            }
        };
        let module_create_info = vk::ShaderModuleCreateInfo::builder().code(code).build();
        let module = match device.create_shader_module(&module_create_info, None) {
            Ok(module) => module,
            Err(err) => return Err(ShaderCreateError::Other(err.to_string())),
        };

        // Name the shader if needed
        if let Some(name) = create_info.debug_name {
            if let Some(debug) = debug {
                let name = CString::new(name).unwrap();
                let name_info = vk::DebugUtilsObjectNameInfoEXT::builder()
                    .object_type(vk::ObjectType::SHADER_MODULE)
                    .object_handle(module.as_raw())
                    .object_name(&name)
                    .build();

                debug
                    .debug_utils_set_object_name(device.handle(), &name_info)
                    .unwrap();
            }
        }

        Ok(Shader { module })
    }
}
