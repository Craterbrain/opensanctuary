use std::sync::Arc;
#[cfg(feature = "wgpu-render")]
use wgpu::{Instance, Device, Queue};

#[derive(Debug, Clone)]
pub struct RenderTargetDimensions {
    pub width: u32,
    pub height: u32,
}

impl Default for RenderTargetDimensions {
    fn default() -> Self {
        Self { width: 1920, height: 1080 }
    }
}

pub struct RenderContext {
    #[cfg(feature = "wgpu-render")]
    pub instance: Instance,
    #[cfg(feature = "wgpu-render")]
    pub device: Device,
    #[cfg(feature = "wgpu-render")]
    pub queue: Queue,
    pub dimensions: RenderTargetDimensions,
}

impl RenderContext {
    #[cfg(feature = "wgpu-render")]
    pub fn new(dimensions: RenderTargetDimensions) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        
        let adapter = pollster::block_on(instance.request_adapter(
            &wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            },
        )).expect("Failed to find an appropriate adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("OS-Next Device"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
            },
            None,
        )).expect("Failed to create device");

        tracing::info!("Real WGPU Device initialized: {:?}", adapter.get_info());

        Self {
            instance,
            device,
            queue,
            dimensions,
        }
    }

    #[cfg(not(feature = "wgpu-render"))]
    pub fn new(dimensions: RenderTargetDimensions) -> Self {
        Self { dimensions }
    }
}

pub struct PresentationRenderer {
    pub context: Arc<RenderContext>,
}

impl PresentationRenderer {
    pub fn new(dimensions: RenderTargetDimensions) -> Self {
        Self {
            context: Arc::new(RenderContext::new(dimensions)),
        }
    }
}
