//! Retained native wgpu resources. No WebView, JavaScript, or unsafe byte casts.
use std::{
    future::Future,
    sync::{atomic::{AtomicU64, Ordering}, Arc},
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};
use eframe::egui::{self, Rect};
use egui_wgpu::{wgpu, Callback, CallbackResources, CallbackTrait, ScreenDescriptor};
use super::boot::BootPose;

#[derive(Default)]
pub(super) struct Receipt {
    pub prepared: AtomicU64,
    pub painted: AtomicU64,
}

struct Gpu {
    pipeline: wgpu::RenderPipeline,
    uniforms: [wgpu::Buffer; 2],
    bindings: [wgpu::BindGroup; 2],
}

pub(super) struct NativeGpu {
    resources: Arc<Gpu>,
    pub receipt: Arc<Receipt>,
    pub label: String,
}

impl NativeGpu {
    pub fn new(state: &egui_wgpu::RenderState) -> Result<Self, String> {
        let device = &state.device;
        // wgpu 30 scopes return guards. Inspect every scope before accepting resources.
        let internal_scope = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let memory_scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let created = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("PWDS native dome, glass tubes and ignition"),
                source: wgpu::ShaderSource::Wgsl(include_str!("pwds.wgsl").into()),
            });
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("PWDS pose uniform layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(96) },
                    count: None,
                }],
            });
            let uniforms = std::array::from_fn(|_| device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("PWDS retained pose buffer"), size: 96,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            let bindings = std::array::from_fn(|i| device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("PWDS retained pose binding"), layout: &layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms[i].as_entire_binding() }],
            }));
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("PWDS native pipeline layout"),
                bind_group_layouts: &[Some(&layout)], immediate_size: 0,
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("PWDS native procedural glass pipeline"), layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"),
                    buffers: &[], compilation_options: wgpu::PipelineCompilationOptions::default() },
                primitive: wgpu::PrimitiveState::default(), depth_stencil: None,
                multisample: wgpu::MultisampleState { count: 1, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if state.target_format.is_srgb() { "fs_linear" } else { "fs_gamma" }),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: state.target_format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })], compilation_options: wgpu::PipelineCompilationOptions::default(),
                }), multiview_mask: None, cache: None,
            });
            Gpu { pipeline, uniforms, bindings }
        }));
        for scope in [validation_scope, memory_scope, internal_scope] {
            if let Some(error) = resolve(device, scope.pop())? {
                return Err(format!("PWDS native GPU resource creation failed: {error}"));
            }
        }
        let resources = created.map_err(|_| "PWDS native GPU resource creation panicked".to_owned())?;
        let adapter = state.adapter.get_info();
        Ok(Self {
            resources: Arc::new(resources), receipt: Arc::new(Receipt::default()),
            label: format!("Native Rust / egui-wgpu 0.36.2 / {:?} / {}", adapter.backend, adapter.name),
        })
    }

    pub fn callback(&self, rect: Rect, pose: BootPose, foreground: bool, epoch: u64) -> egui::PaintCallback {
        Callback::new_paint_callback(rect, Pass {
            gpu: self.resources.clone(), receipt: self.receipt.clone(),
            pose, foreground, epoch, rect,
        })
    }
}

struct Pass {
    gpu: Arc<Gpu>, receipt: Arc<Receipt>, pose: BootPose,
    foreground: bool, epoch: u64, rect: Rect,
}

impl CallbackTrait for Pass {
    fn prepare(&self, _device: &wgpu::Device, queue: &wgpu::Queue,
        screen: &ScreenDescriptor, _encoder: &mut wgpu::CommandEncoder,
        _resources: &mut CallbackResources) -> Vec<wgpu::CommandBuffer> {
        let p = self.pose;
        let values = [
            self.rect.width(), self.rect.height(), screen.pixels_per_point, p.seconds,
            p.frame, p.net, p.ignite, p.core,
            p.flash, p.particles, p.shockwave, p.emblem,
            p.fly, p.content, if self.foreground { 1.0 } else { 0.0 }, 0.0,
            self.rect.left(), self.rect.top(), 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0,
        ];
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_ne_bytes()).collect();
        queue.write_buffer(&self.gpu.uniforms[self.foreground as usize], 0, &bytes);
        if self.foreground { self.receipt.prepared.store(self.epoch, Ordering::Release); }
        Vec::new()
    }

    fn paint(&self, info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>,
        _resources: &CallbackResources) {
        pass.set_pipeline(&self.gpu.pipeline);
        pass.set_bind_group(0, &self.gpu.bindings[self.foreground as usize], &[]);
        if self.foreground {
            // Only the left rail and upper-right corner contain front pipes.
            // Keep the full viewport projection but shade only those scissors.
            let viewport=info.viewport;
            let regions=[
                Rect::from_min_max(viewport.min,egui::pos2(viewport.left()+viewport.width()*0.15,viewport.bottom())),
                Rect::from_min_max(egui::pos2(viewport.left()+viewport.width()*0.74,viewport.top()),
                    egui::pos2(viewport.right(),viewport.top()+viewport.height()*0.22)),
            ];
            for region in regions {
                let region=region.intersect(info.clip_rect);
                let px=|v: f32,max: u32|(v*info.pixels_per_point).round().clamp(0.0,max as f32) as u32;
                let left=px(region.left(),info.screen_size_px[0]);let right=px(region.right(),info.screen_size_px[0]);
                let top=px(region.top(),info.screen_size_px[1]);let bottom=px(region.bottom(),info.screen_size_px[1]);
                if right>left && bottom>top { pass.set_scissor_rect(left,top,right-left,bottom-top);pass.draw(0..3,0..1); }
            }
            let clip=info.clip_rect_in_pixels();
            if clip.width_px>0 && clip.height_px>0 {
                pass.set_scissor_rect(clip.left_px as u32,clip.top_px as u32,clip.width_px as u32,clip.height_px as u32);
            }
        } else { pass.draw(0..3,0..1); }
        // This acknowledgement occurs after egui has prepared texture updates and
        // recorded real content draws, not when the UI merely enqueues a callback.
        // It proves command recording; GPU completion/presentation is the host's job.
        if self.foreground { self.receipt.painted.store(self.epoch, Ordering::Release); }
    }
}

struct ThreadWake(std::thread::Thread);
impl Wake for ThreadWake { fn wake(self: Arc<Self>) { self.0.unpark(); } }

fn resolve<T>(device: &wgpu::Device, future: impl Future<Output = T>) -> Result<T, String> {
    let mut future = std::pin::pin!(future);
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) { return Ok(value); }
        device.poll(wgpu::PollType::Poll).map_err(|e| format!("PWDS GPU validation poll failed: {e}"))?;
        if Instant::now() >= deadline { return Err("PWDS GPU validation did not finish within five seconds".into()); }
        std::thread::park_timeout(Duration::from_millis(1));
    }
}
