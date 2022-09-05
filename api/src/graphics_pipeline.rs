use std::sync::Arc;

use crate::{context::Context, shader::Shader, types::*, Backend};
use thiserror::Error;

#[derive(Clone)]
pub struct ShaderStages<B: Backend> {
    pub vertex: Shader<B>,
    pub fragment: Option<Shader<B>>,
}

#[derive(Clone)]
pub struct VertexInputAttribute {
    pub location: u32,
    pub binding: u32,
    pub format: VertexFormat,
    pub offset: u32,
}

#[derive(Clone)]
pub struct VertexInputBinding {
    pub binding: u32,
    pub stride: u32,
    pub input_rate: VertexInputRate,
}

#[derive(Clone)]
pub struct VertexInputState {
    pub attributes: Vec<VertexInputAttribute>,
    pub bindings: Vec<VertexInputBinding>,
    pub topology: PrimitiveTopology,
}

#[derive(Clone)]
pub struct RasterizationState {
    pub polygon_mode: PolygonMode,
    pub cull_mode: CullMode,
    pub front_face: FrontFace,
}

#[derive(Clone)]
pub struct DepthStencilState {
    pub depth_clamp: bool,
    pub depth_test: bool,
    pub depth_write: bool,
    pub depth_compare: CompareOp,
    pub min_depth: f32,
    pub max_depth: f32,
}

#[derive(Clone)]
pub struct ColorBlendAttachment {
    pub write_mask: ColorComponents,
    pub blend: bool,
    pub color_blend_op: BlendOp,
    pub src_color_blend_factor: BlendFactor,
    pub dst_color_blend_factor: BlendFactor,
    pub alpha_blend_op: BlendOp,
    pub src_alpha_blend_factor: BlendFactor,
    pub dst_alpha_blend_factor: BlendFactor,
}

#[derive(Default, Clone)]
pub struct ColorBlendState {
    pub attachments: Vec<ColorBlendAttachment>,
}

#[derive(Clone)]
pub struct GraphicsPipelineCreateInfo<B: Backend> {
    pub stages: ShaderStages<B>,
    pub vertex_input: VertexInputState,
    pub rasterization: RasterizationState,
    pub depth_stencil: Option<DepthStencilState>,
    pub color_blend: Option<ColorBlendState>,
}

pub struct GraphicsPipeline<B: Backend>(pub(crate) Arc<GraphicsPipelineInner<B>>);

pub(crate) struct GraphicsPipelineInner<B: Backend> {
    ctx: Context<B>,
    pub(crate) id: B::GraphicsPipeline,
}

#[derive(Debug, Error)]
pub enum GraphicsPipelineCreateError {
    #[error("no vertex attributes or bindings were provided")]
    NoAttributesOrBindings,
    #[error("no depth/stencil or color attachments provided")]
    NoAttachments,
    #[error("an error occured: {0}")]
    Other(String),
}

impl<B: Backend> GraphicsPipeline<B> {
    pub fn new(
        ctx: Context<B>,
        create_info: GraphicsPipelineCreateInfo<B>,
    ) -> Result<Self, GraphicsPipelineCreateError> {
        let id = unsafe { ctx.0.create_graphics_pipeline(create_info)? };
        Ok(Self(Arc::new(GraphicsPipelineInner { ctx, id })))
    }

    #[inline(always)]
    pub fn internal(&self) -> &B::GraphicsPipeline {
        &self.0.id
    }
}

impl<B: Backend> Clone for GraphicsPipeline<B> {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<B: Backend> Drop for GraphicsPipelineInner<B> {
    fn drop(&mut self) {
        unsafe {
            self.ctx.0.destroy_graphics_pipeline(&mut self.id);
        }
    }
}

impl Default for VertexInputState {
    #[inline(always)]
    fn default() -> Self {
        Self {
            attributes: Vec::default(),
            bindings: Vec::default(),
            topology: PrimitiveTopology::TriangleList,
        }
    }
}

impl Default for RasterizationState {
    #[inline(always)]
    fn default() -> Self {
        Self {
            polygon_mode: PolygonMode::Fill,
            cull_mode: CullMode::Back,
            front_face: FrontFace::CounterClockwise,
        }
    }
}

impl Default for DepthStencilState {
    #[inline(always)]
    fn default() -> Self {
        Self {
            depth_clamp: false,
            depth_test: false,
            depth_write: false,
            depth_compare: CompareOp::Always,
            min_depth: 0.0,
            max_depth: 1.0,
        }
    }
}

impl Default for ColorBlendAttachment {
    #[inline(always)]
    fn default() -> Self {
        Self {
            write_mask: ColorComponents::empty(),
            blend: false,
            color_blend_op: BlendOp::Add,
            src_color_blend_factor: BlendFactor::One,
            dst_color_blend_factor: BlendFactor::Zero,
            alpha_blend_op: BlendOp::Add,
            src_alpha_blend_factor: BlendFactor::One,
            dst_alpha_blend_factor: BlendFactor::Zero,
        }
    }
}
