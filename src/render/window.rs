use std::sync::Arc;
use wgpu::{Instance, Surface, Device, Queue, SurfaceConfiguration};
use tao::window::{Window, WindowBuilder};
use tao::event_loop::EventLoopWindowTarget;

pub struct ProjectionState {
    pub window: Arc<Window>,
    pub surface: Surface<'static>,
    pub device: Device,
    pub queue: Queue,
    pub config: SurfaceConfiguration,
}

impl ProjectionState {
    pub async fn new<T>(event_loop: &EventLoopWindowTarget<T>) -> Self {
        let window = Arc::new(
            WindowBuilder::new()
                .with_title("OS-Next Projection Output")
                .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 720.0))
                // .with_fullscreen(Some(tao::window::Fullscreen::Borderless(None))) // Too intrusive for debugging
                .build(event_loop)
                .expect("Failed to create projection window"),
        );

        let instance = Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone()).expect("Failed to create surface");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("Failed to find wgpu adapter");

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Projection Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: adapter.limits(),
                },
                None,
            )
            .await
            .expect("Failed to create device");

        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
        let max_dim = device.limits().max_texture_dimension_2d;

        let width = size.width.clamp(1, max_dim);
        let height = size.height.clamp(1, max_dim);

        let config = SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        tracing::info!("WGPU Projection Window initialized ({}x{}, max_texture_2d: {})", width, height, max_dim);

        Self {
            window,
            surface,
            device,
            queue,
            config,
        }
    }

    pub fn resize(&mut self, new_size: tao::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            let max_dim = self.device.limits().max_texture_dimension_2d;
            self.config.width = new_size.width.clamp(1, max_dim);
            self.config.height = new_size.height.clamp(1, max_dim);
            self.surface.configure(&self.device, &self.config);
        }
    }

    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Let's clear to a nice dark blue to prove wgpu is working
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1,
                            g: 0.2,
                            b: 0.3,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}
