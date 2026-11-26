use parking_lot::RwLock;
use windows::{
    core::{h, Interface, HSTRING, PCWSTR},
    Foundation::{IPropertyValue, PropertyValue},
    Graphics::Effects::{
        IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectSource_Impl, IGraphicsEffect_Impl,
    },
    Win32::{
        Foundation::E_INVALIDARG,
        Graphics::{
            Direct2D::{
                CLSID_D2D1GaussianBlur, CLSID_D2D1Saturation, Common::D2D1_BORDER_MODE_HARD,
                D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED,
            },
            Dxgi::IDXGISwapChain1,
        },
        System::WinRT::{
            Composition::ICompositorInterop,
            Graphics::Direct2D::{
                IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl,
                GRAPHICS_EFFECT_PROPERTY_MAPPING, GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT,
            },
        },
    },
    UI::Composition::{
        CompositionBackdropBrush, CompositionBitmapInterpolationMode, CompositionEffectBrush,
        CompositionStretch, CompositionSurfaceBrush, Compositor,
    },
};
use windows_core::implement;

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
pub(super) struct D2dGaussianBlurEffect {
    name: RwLock<HSTRING>,
    std_dev: f32,
    source: IGraphicsEffectSource,
}

impl IGraphicsEffectSource_Impl for D2dGaussianBlurEffect_Impl {}

impl IGraphicsEffect_Impl for D2dGaussianBlurEffect_Impl {
    fn Name(&self) -> windows_core::Result<HSTRING> {
        Ok(self.name.read().clone())
    }

    fn SetName(&self, name: &HSTRING) -> windows_core::Result<()> {
        *self.name.write() = name.clone();
        Ok(())
    }
}

impl IGraphicsEffectD2D1Interop_Impl for D2dGaussianBlurEffect_Impl {
    fn GetEffectId(&self) -> windows_core::Result<windows_core::GUID> {
        Ok(CLSID_D2D1GaussianBlur)
    }

    fn GetNamedPropertyMapping(
        &self,
        name: &PCWSTR,
        index: *mut u32,
        mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> windows_core::Result<()> {
        if index.is_null() || mapping.is_null() {
            return Err(windows_core::Error::from(E_INVALIDARG));
        }
        let prop = unsafe { name.to_string() }.unwrap_or_default();
        if prop.eq_ignore_ascii_case("BlurAmount") {
            unsafe {
                *index = 0;
                *mapping = GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT;
            }
            return Ok(());
        }
        Err(windows_core::Error::from(E_INVALIDARG))
    }

    fn GetPropertyCount(&self) -> windows_core::Result<u32> {
        Ok(3)
    }

    fn GetProperty(&self, index: u32) -> windows_core::Result<IPropertyValue> {
        match index {
            // D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION = 0
            0 => PropertyValue::CreateSingle(self.std_dev)?.cast(),
            // D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION = 1
            1 => PropertyValue::CreateUInt32(D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED.0 as u32)?
                .cast(),
            // D2D1_GAUSSIANBLUR_PROP_BORDER_MODE = 2 (HARD hindrer kant-fade)
            2 => PropertyValue::CreateUInt32(D2D1_BORDER_MODE_HARD.0 as u32)?.cast(),
            _ => Err(windows_core::Error::from(E_INVALIDARG)),
        }
    }

    fn GetSource(&self, index: u32) -> windows_core::Result<IGraphicsEffectSource> {
        if index == 0 {
            Ok(self.source.clone())
        } else {
            Err(windows_core::Error::from(E_INVALIDARG))
        }
    }

    fn GetSourceCount(&self) -> windows_core::Result<u32> {
        Ok(1)
    }
}

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
pub(super) struct D2dSaturationEffect {
    name: RwLock<HSTRING>,
    saturation: f32,
    source: IGraphicsEffectSource,
}

impl IGraphicsEffectSource_Impl for D2dSaturationEffect_Impl {}

impl IGraphicsEffect_Impl for D2dSaturationEffect_Impl {
    fn Name(&self) -> windows_core::Result<HSTRING> {
        Ok(self.name.read().clone())
    }

    fn SetName(&self, name: &HSTRING) -> windows_core::Result<()> {
        *self.name.write() = name.clone();
        Ok(())
    }
}

impl IGraphicsEffectD2D1Interop_Impl for D2dSaturationEffect_Impl {
    fn GetEffectId(&self) -> windows_core::Result<windows_core::GUID> {
        Ok(CLSID_D2D1Saturation)
    }

    fn GetNamedPropertyMapping(
        &self,
        name: &PCWSTR,
        index: *mut u32,
        mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> windows_core::Result<()> {
        if index.is_null() || mapping.is_null() {
            return Err(windows_core::Error::from(E_INVALIDARG));
        }
        let prop = unsafe { name.to_string() }.unwrap_or_default();
        if prop.eq_ignore_ascii_case("Saturation") {
            unsafe {
                *index = 0;
                *mapping = GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT;
            }
            return Ok(());
        }
        Err(windows_core::Error::from(E_INVALIDARG))
    }

    fn GetPropertyCount(&self) -> windows_core::Result<u32> {
        Ok(1)
    }

    fn GetProperty(&self, index: u32) -> windows_core::Result<IPropertyValue> {
        match index {
            // D2D1_SATURATION_PROP_SATURATION = 0
            0 => PropertyValue::CreateSingle(self.saturation)?.cast(),
            _ => Err(windows_core::Error::from(E_INVALIDARG)),
        }
    }

    fn GetSource(&self, index: u32) -> windows_core::Result<IGraphicsEffectSource> {
        if index == 0 {
            Ok(self.source.clone())
        } else {
            Err(windows_core::Error::from(E_INVALIDARG))
        }
    }

    fn GetSourceCount(&self) -> windows_core::Result<u32> {
        Ok(1)
    }
}

pub(super) fn build_surface_brush(
    compositor: &Compositor,
    swap_chain: &IDXGISwapChain1,
) -> windows_core::Result<CompositionSurfaceBrush> {
    let interop: ICompositorInterop = compositor.cast()?;
    let surface = unsafe { interop.CreateCompositionSurfaceForSwapChain(swap_chain)? };
    let brush = compositor.CreateSurfaceBrushWithSurface(&surface)?;
    brush.SetStretch(CompositionStretch::Fill)?;
    brush.SetBitmapInterpolationMode(CompositionBitmapInterpolationMode::Linear)?;
    Ok(brush)
}

pub(super) fn build_effect_brush(
    compositor: &Compositor,
    sigma: f32,
    sat: f32,
) -> windows_core::Result<CompositionEffectBrush> {
    let source_param =
        windows::UI::Composition::CompositionEffectSourceParameter::Create(h!("source"))?;
    let param_source: IGraphicsEffectSource = source_param.cast()?;

    let blur = D2dGaussianBlurEffect {
        name: RwLock::new(HSTRING::from("Blur")),
        std_dev: sigma,
        source: param_source,
    };
    let blur_source: IGraphicsEffectSource = blur.into();

    let root_effect: IGraphicsEffect = if (sat - 1.0).abs() > 0.005 {
        let sat_fx = D2dSaturationEffect {
            name: RwLock::new(HSTRING::from("Saturation")),
            saturation: sat,
            source: blur_source,
        };
        let sat_source: IGraphicsEffectSource = sat_fx.into();
        sat_source.cast()?
    } else {
        blur_source.cast()?
    };

    let factory = compositor.CreateEffectFactory(&root_effect)?;
    let brush = factory.CreateBrush()?;
    let host_backdrop: CompositionBackdropBrush = compositor.CreateHostBackdropBrush()?;
    brush.SetSourceParameter(h!("source"), &host_backdrop)?;
    Ok(brush)
}
