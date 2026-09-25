use crate::core::models::{ShowState, Slide, Theme};
use crate::render::pipeline::RenderTargetDimensions;

#[derive(Debug, Clone)]
pub struct ComposedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba_data: Vec<u8>,
    pub is_blackout: bool,
    pub is_clear: bool,
}

pub struct LayerCompositor {
    dimensions: RenderTargetDimensions,
}

impl LayerCompositor {
    pub fn new(dimensions: RenderTargetDimensions) -> Self {
        Self { dimensions }
    }

    pub fn dimensions(&self) -> &RenderTargetDimensions {
        &self.dimensions
    }

    pub fn render_solid_color(&self, r: u8, g: u8, b: u8, a: u8) -> ComposedFrame {
        let pixel_count = (self.dimensions.width * self.dimensions.height) as usize;
        let mut rgba = Vec::with_capacity(pixel_count * 4);
        for _ in 0..pixel_count {
            rgba.push(r);
            rgba.push(g);
            rgba.push(b);
            rgba.push(a);
        }

        ComposedFrame {
            width: self.dimensions.width,
            height: self.dimensions.height,
            rgba_data: rgba,
            is_blackout: false,
            is_clear: false,
        }
    }

    pub fn compose_live_slide(
        &self,
        state: &ShowState,
        _theme: Option<&Theme>,
    ) -> ComposedFrame {
        if state.is_blackout {
            return self.render_solid_color(0, 0, 0, 255);
        }
        self.render_solid_color(16, 32, 39, 255)
    }

    pub fn compose_stage_slide(
        &self,
        current_slide: Option<&Slide>,
        next_slide: Option<&Slide>,
        clock_str: &str,
    ) -> (String, String, String) {
        let curr_text = current_slide.map(|s| s.text.as_str()).unwrap_or("(No Live Content)").to_string();
        let next_text = next_slide.map(|s| s.text.as_str()).unwrap_or("(End of Item)").to_string();
        (curr_text, next_text, clock_str.to_string())
    }
}
