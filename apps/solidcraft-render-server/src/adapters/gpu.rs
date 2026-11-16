//! The headless graphics device: an adapter picked with no surface, its device, and egui-wgpu's
//! `RenderState`. With no surface the target format is `Rgba8Unorm`
//! (`vendor/egui-wgpu/src/lib.rs:476-482`, `preferred_framebuffer_format` `:679-695`), so read-back
//! bytes are RGBA in gamma space, as a window would show them.

use std::sync::Arc;

use solidcraft_viewport_gpu::select::{self, Policy};

/// The scene's anti-aliasing asked for, as the desktop app's default (`apps/solidcraft/src/main.rs`, `MSAA`).
pub const SCENE_MSAA: u32 = 4;
/// The section cap marks where it shows in the stencil (`apps/solidcraft/src/main.rs`, `STENCIL_BITS`).
pub const STENCIL_BITS: u8 = 8;

/// The device and what was chosen, for `hello`, `ready` and the UI's own setup.
pub struct Gpu {
    pub rs: egui_wgpu::RenderState,
    /// `hello.backend`.
    pub backend: Option<&'static str>,
    pub adapter: String,
    pub software: bool,
    /// The scene's depth buffer: 32-bit float with stencil where the device has it, else 24.
    pub depth_bits: u8,
    /// The sample counts the scene can be drawn with.
    pub supported_samples: Vec<u32>,
    /// The scene's sample count asked for (the UI's own check may still fall back to 1).
    pub scene_samples: u32,
}

/// Open the device on the adapter `policy` admits: hardware before software, then the platform's
/// preferred backend (Windows D3D12 first, as `apps/solidcraft/src/startup.rs:435` `WINDOWS_ORDER`).
pub fn open(policy: Policy) -> Result<Gpu, String> {
    // `vendor/egui-wgpu/src/setup.rs:241`: an instance with no display handle (headless).
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    // Metal on macOS, D3D12 and Vulkan on Windows, Vulkan and GL elsewhere; `WGPU_BACKEND` narrows
    // them (`crates/viewport-gpu/src/select.rs:120`).
    setup.instance_descriptor.backends = select::native_backends();
    // Never a `dxcompiler.dll` another program put on PATH (`crates/viewport-gpu/src/lib.rs:90`, #6756).
    setup.instance_descriptor.backend_options.dx12.shader_compiler = solidcraft_viewport_gpu::dx12_compiler();
    // Intel Macs refuse wgpu's indirect-call validation shader (`apps/solidcraft/src/startup.rs`,
    // `window_instance_flags`).
    if cfg!(all(target_os = "macos", target_arch = "x86_64")) && std::env::var_os("WGPU_VALIDATION_INDIRECT_CALL").is_none() {
        setup.instance_descriptor.flags -= wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL;
    }
    // `NativeAdapterSelectorMethod` (`vendor/egui-wgpu/src/setup.rs:155`): `Ok(None)` would fall
    // back to wgpu's own request, which ignores the policy, so a refusal is an error here.
    setup.native_adapter_selector = Some(Arc::new(move |adapters, _surface| choose(adapters, policy).map(Some)));
    // A float depth buffer with a stencil wherever the adapter has it, as the desktop app asks
    // (`apps/solidcraft/src/main.rs:1641-1646`; `crates/viewport-gpu/src/lib.rs:42`).
    let base = setup.device_descriptor.clone();
    setup.device_descriptor = Arc::new(move |adapter| {
        let mut d = base(adapter);
        d.required_features |= solidcraft_viewport_gpu::depth_features(adapter);
        d
    });
    let config = egui_wgpu::WgpuConfiguration { wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup), ..Default::default() };
    // `vendor/egui-wgpu/src/setup.rs:97`.
    let instance = pollster::block_on(config.wgpu_setup.new_instance());
    // The UI's own pass: one sample, no depth; the viewport draws its scene offscreen with its own
    // depth and MSAA and blits it (`apps/solidcraft/src/main.rs:1251-1257`).
    let options = egui_wgpu::RendererOptions { msaa_samples: 1, depth_stencil_format: None, ..Default::default() };
    // `vendor/egui-wgpu/src/lib.rs:389`: no surface; also the device-lost watch (`:471`).
    let rs = pollster::block_on(egui_wgpu::RenderState::create(&config, &instance, None, options)).map_err(|e| format!("graphics device: {e}"))?;
    if !matches!(rs.target_format, wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm) {
        return Err(format!("graphics device: unexpected target format {:?}", rs.target_format));
    }
    let info = rs.adapter.get_info();
    let depth_bits = if rs.device.features().contains(solidcraft_viewport_gpu::FLOAT_DEPTH_STENCIL) { 32 } else { 24 };
    // `vendor/egui-wgpu/src/lib.rs:823`; the rule the desktop and web apps share (`crates/ui-egui/src/gpu.rs:110`).
    let supported_samples =
        egui_wgpu::supported_msaa_samples(&rs.adapter, &rs.device, rs.target_format, egui_wgpu::depth_format_from_bits(depth_bits, STENCIL_BITS));
    let scene_samples = solidcraft_ui_egui::gpu::scene_samples(info.backend, SCENE_MSAA, &supported_samples);
    log::info!(
        "graphics: {:?} {} ({:?}), depth {depth_bits}, scene MSAA {scene_samples}x of {supported_samples:?}",
        info.backend,
        info.name,
        info.device_type
    );
    Ok(Gpu {
        backend: backend_name(info.backend),
        adapter: info.name.clone(),
        software: info.device_type == wgpu::DeviceType::Cpu,
        depth_bits,
        supported_samples,
        scene_samples,
        rs,
    })
}

/// The adapter `policy` admits, best first: a discrete GPU, an integrated one, a virtual one, an
/// unknown kind, a software rasterizer; among equals the platform's preferred backend.
fn choose(adapters: &[wgpu::Adapter], policy: Policy) -> Result<wgpu::Adapter, String> {
    adapters
        .iter()
        .filter(|a| policy.admits(select::tier_of(a.get_info().device_type)))
        .max_by_key(|a| {
            let info = a.get_info();
            (rank(info.device_type), backend_preference(info.backend))
        })
        .cloned()
        .ok_or_else(|| {
            let found: Vec<String> =
                adapters.iter().map(|a| format!("{} ({:?}, {:?})", a.get_info().name, a.get_info().backend, a.get_info().device_type)).collect();
            format!(
                "no graphics adapter qualifies under `{}` (found: {})",
                policy.name(),
                if found.is_empty() { "none".into() } else { found.join(", ") }
            )
        })
}

fn rank(t: wgpu::DeviceType) -> u8 {
    match t {
        wgpu::DeviceType::DiscreteGpu => 4,
        wgpu::DeviceType::IntegratedGpu => 3,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 1,
        wgpu::DeviceType::Cpu => 0,
    }
}

fn backend_preference(b: wgpu::Backend) -> u8 {
    match b {
        wgpu::Backend::Dx12 if cfg!(target_os = "windows") => 3,
        wgpu::Backend::Metal => 3,
        wgpu::Backend::Vulkan => 2,
        wgpu::Backend::Gl => 1,
        _ => 0,
    }
}

fn backend_name(b: wgpu::Backend) -> Option<&'static str> {
    match b {
        wgpu::Backend::Vulkan => Some("vulkan"),
        wgpu::Backend::Dx12 => Some("dx12"),
        wgpu::Backend::Gl => Some("gl"),
        wgpu::Backend::Metal => Some("metal"),
        _ => None,
    }
}
