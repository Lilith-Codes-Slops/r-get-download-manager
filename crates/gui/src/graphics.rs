//! Conservative native GPU selection for the existing wgpu renderer.
use std::sync::Arc;

use eframe::egui_wgpu::WgpuSetup;
use eframe::wgpu::{Backend, DeviceType};

// Keep enough space for large glyph sets and high-DPI text without starting an
// 8192-pixel-wide atlas on every device. egui grows its atlas on demand.
pub const FONT_ATLAS_MAX_SIDE: usize = 4096;

fn rank(device: DeviceType, backend: Backend) -> (u8, u8) {
    let device_rank = match device {
        DeviceType::IntegratedGpu => 0,
        DeviceType::DiscreteGpu => 1,
        DeviceType::VirtualGpu | DeviceType::Other => 2,
        DeviceType::Cpu => 3,
    };
    // On Linux, prefer Vulkan for the same kind of GPU, but retain GL as a
    // fallback. Other platforms keep wgpu's adapter enumeration order.
    let backend_rank = if cfg!(target_os = "linux") && backend != Backend::Vulkan {
        1
    } else {
        0
    };
    (device_rank, backend_rank)
}

pub fn native_options() -> eframe::NativeOptions {
    let mut options = eframe::NativeOptions::default();
    if let WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        let device_descriptor = Arc::clone(&setup.device_descriptor);
        setup.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
            if std::env::var_os("RGET_GPU_DIAGNOSTICS").is_some() {
                eprintln!("Graphics candidates: {}", adapters.len());
                for adapter in adapters {
                    let info = adapter.get_info();
                    let descriptor = device_descriptor(adapter);
                    let surface_ok =
                        surface.is_none_or(|surface| adapter.is_surface_supported(surface));
                    let limits = adapter.limits();
                    let limits_ok = descriptor.required_limits.check_limits(&limits);
                    let features_ok = adapter.features().contains(descriptor.required_features);
                    eprintln!(
                        "  {} ({:?}, {:?}): surface={}, limits={}, features={}",
                        info.name,
                        info.device_type,
                        info.backend,
                        surface_ok,
                        limits_ok,
                        features_ok
                    );
                    if !limits_ok {
                        descriptor.required_limits.check_limits_with_fail_fn(
                            &limits,
                            false,
                            |name, required, supported| {
                                eprintln!("    {name}: required {required}, supported {supported}");
                            },
                        );
                    }
                }
            }
            let selected = adapters
                .iter()
                .filter(|adapter| {
                    let descriptor = device_descriptor(adapter);
                    surface.is_none_or(|surface| adapter.is_surface_supported(surface))
                        && descriptor.required_limits.check_limits(&adapter.limits())
                        && adapter.features().contains(descriptor.required_features)
                })
                .min_by_key(|adapter| {
                    let info = adapter.get_info();
                    rank(info.device_type, info.backend)
                })
                .ok_or_else(|| "No compatible graphics adapter found for the window".to_owned())?;
            let info = selected.get_info();
            eprintln!(
                "Graphics adapter: {} ({:?}, {:?})",
                info.name, info.device_type, info.backend
            );
            Ok(selected.clone())
        }));
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrated_hardware_precedes_discrete_and_software() {
        assert!(
            rank(DeviceType::IntegratedGpu, Backend::Gl)
                < rank(DeviceType::DiscreteGpu, Backend::Vulkan)
        );
        assert!(
            rank(DeviceType::DiscreteGpu, Backend::Gl) < rank(DeviceType::Cpu, Backend::Vulkan)
        );
        assert!(rank(DeviceType::Other, Backend::Gl) < rank(DeviceType::Cpu, Backend::Vulkan));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_prefers_vulkan_within_the_same_device_type() {
        assert!(
            rank(DeviceType::IntegratedGpu, Backend::Vulkan)
                < rank(DeviceType::IntegratedGpu, Backend::Gl)
        );
    }
}
