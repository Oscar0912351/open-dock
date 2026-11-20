use std::{fs, path::Path, ptr};

use image::{Rgb, RgbImage};
use windows::{
    core::{Interface, PCSTR},
    Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::{
                Fxc::{D3DCompile, D3DCOMPILE_ENABLE_STRICTNESS, D3DCOMPILE_OPTIMIZATION_LEVEL3},
                ID3DBlob, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
                D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            },
            Direct3D11::{
                D3D11CreateDevice, ID3D11Buffer, ID3D11Device, ID3D11DeviceContext,
                ID3D11PixelShader, ID3D11RenderTargetView, ID3D11Resource, ID3D11Texture2D,
                ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_RENDER_TARGET,
                D3D11_BUFFER_DESC, D3D11_CPU_ACCESS_READ, D3D11_CPU_ACCESS_WRITE,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
                D3D11_MAP_WRITE_DISCARD, D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA,
                D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_DYNAMIC,
                D3D11_USAGE_STAGING, D3D11_VIEWPORT,
            },
            Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
        },
    },
};

use crate::error::AppError;

const COMMON_HLSL: &str = include_str!("shaders/common.hlsl");
const SHADER_NORDIC_AURORA: &str = include_str!("shaders/nordic_aurora.hlsl");
const SHADER_OBSIDIAN_SILK: &str = include_str!("shaders/obsidian_silk.hlsl");
const SHADER_SOLAR_HORIZON: &str = include_str!("shaders/solar_horizon.hlsl");
const SHADER_ALPINE_MIST: &str = include_str!("shaders/alpine_mist.hlsl");
const SHADER_MONOLITH_MESH: &str = include_str!("shaders/monolith_mesh.hlsl");
const SHADER_DEEP_OCEAN: &str = include_str!("shaders/deep_ocean.hlsl");
const SHADER_AMBER_DUNES: &str = include_str!("shaders/amber_dunes.hlsl");
const SHADER_PRISM_GLASS: &str = include_str!("shaders/prism_glass.hlsl");

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SceneUniforms {
    pub resolution: [f32; 2],
    pub time: f32,
    pub brightness: f32,
    pub accent: [f32; 4],
}

pub fn scene_hlsl_source(shader_id: &str) -> Option<&'static str> {
    match shader_id {
        "scene-nordic-aurora" => Some(SHADER_NORDIC_AURORA),
        "scene-obsidian-silk" => Some(SHADER_OBSIDIAN_SILK),
        "scene-solar-horizon" => Some(SHADER_SOLAR_HORIZON),
        "scene-alpine-mist" => Some(SHADER_ALPINE_MIST),
        "scene-monolith-mesh" => Some(SHADER_MONOLITH_MESH),
        "scene-deep-ocean" => Some(SHADER_DEEP_OCEAN),
        "scene-amber-dunes" => Some(SHADER_AMBER_DUNES),
        "scene-prism-glass" => Some(SHADER_PRISM_GLASS),
        _ => None,
    }
}

pub fn compile_hlsl(source: &str, entry_cstr: &[u8], target_cstr: &[u8], debug_name: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut code_blob: Option<ID3DBlob> = None;
    let mut err_blob: Option<ID3DBlob> = None;
    let flags = D3DCOMPILE_ENABLE_STRICTNESS | D3DCOMPILE_OPTIMIZATION_LEVEL3;

    let res = unsafe {
        D3DCompile(
            source.as_ptr() as *const _,
            source.len(),
            PCSTR(debug_name.as_ptr()),
            None,
            None,
            PCSTR(entry_cstr.as_ptr()),
            PCSTR(target_cstr.as_ptr()),
            flags,
            0,
            &mut code_blob,
            Some(&mut err_blob),
        )
    };

    if let Err(err) = res {
        let diag = err_blob
            .as_ref()
            .map(|b| unsafe {
                let ptr = b.GetBufferPointer() as *const u8;
                let len = b.GetBufferSize();
                if ptr.is_null() || len == 0 {
                    String::new()
                } else {
                    String::from_utf8_lossy(std::slice::from_raw_parts(ptr, len)).into_owned()
                }
            })
            .unwrap_or_default();
        return Err(AppError::Other(format!(
            "HLSL-kompilering fejlede ({err}): {}",
            diag.trim()
        )));
    }

    let blob = code_blob
        .ok_or_else(|| AppError::Other("HLSL-kompilering returnerede ingen bytecode".into()))?;
    let bytes = unsafe {
        let ptr = blob.GetBufferPointer() as *const u8;
        let len = blob.GetBufferSize();
        if ptr.is_null() || len == 0 {
            return Err(AppError::Other("Tom HLSL-bytecode".into()));
        }
        std::slice::from_raw_parts(ptr, len).to_vec()
    };
    Ok(bytes)
}

pub fn compile_vertex_shader() -> Result<Vec<u8>, AppError> {
    compile_hlsl(COMMON_HLSL, b"VSMain\0", b"vs_5_0\0", b"common.hlsl\0")
}

pub fn compile_scene_pixel_shader(shader_id: &str) -> Result<Vec<u8>, AppError> {
    let body = scene_hlsl_source(shader_id)
        .ok_or_else(|| AppError::Other(format!("Ukendt shader-scene: {shader_id}")))?;
    let full = format!("{COMMON_HLSL}\n{body}");
    let debug_name = format!("{shader_id}.hlsl\0");
    compile_hlsl(&full, b"PSMain\0", b"ps_5_0\0", debug_name.as_bytes())
}

pub fn render_shader_to_png(
    shader_id: &str,
    width: u32,
    height: u32,
    time_s: f32,
    brightness: f32,
    out_path: &Path,
) -> Result<(), AppError> {
    let w = width.clamp(64, 3840);
    let h = height.clamp(64, 2160);

    let vs_bytes = compile_vertex_shader()?;
    let ps_bytes = compile_scene_pixel_shader(shader_id)?;

    let (device, context) = create_offscreen_d3d11_device()?;

    let mut vs: Option<ID3D11VertexShader> = None;
    let mut ps: Option<ID3D11PixelShader> = None;
    unsafe {
        device.CreateVertexShader(&vs_bytes, None, Some(&mut vs))?;
        device.CreatePixelShader(&ps_bytes, None, Some(&mut ps))?;
    }
    let vs = vs.ok_or_else(|| AppError::Other("Kunne ikke oprette vertex shader".into()))?;
    let ps = ps.ok_or_else(|| AppError::Other("Kunne ikke oprette pixel shader".into()))?;

    let uniforms = SceneUniforms {
        resolution: [w as f32, h as f32],
        time: time_s,
        brightness,
        accent: [1.0, 1.0, 1.0, 1.0],
    };

    let cbuf_desc = D3D11_BUFFER_DESC {
        ByteWidth: std::mem::size_of::<SceneUniforms>() as u32,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let init_data = D3D11_SUBRESOURCE_DATA {
        pSysMem: &uniforms as *const SceneUniforms as *const _,
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut cbuf: Option<ID3D11Buffer> = None;
    unsafe {
        device.CreateBuffer(&cbuf_desc, Some(&init_data), Some(&mut cbuf))?;
    }
    let cbuf = cbuf.ok_or_else(|| AppError::Other("Kunne ikke oprette shader-konstanter".into()))?;

    let tex_desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut rt_tex: Option<ID3D11Texture2D> = None;
    unsafe {
        device.CreateTexture2D(&tex_desc, None, Some(&mut rt_tex))?;
    }
    let rt_tex = rt_tex.ok_or_else(|| AppError::Other("Kunne ikke oprette render-mål".into()))?;

    let rt_res: ID3D11Resource = rt_tex.cast()?;
    let mut rtv: Option<ID3D11RenderTargetView> = None;
    unsafe {
        device.CreateRenderTargetView(&rt_res, None, Some(&mut rtv))?;
    }
    let rtv = rtv.ok_or_else(|| AppError::Other("Kunne ikke oprette render target view".into()))?;

    let staging_desc = D3D11_TEXTURE2D_DESC {
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        ..tex_desc
    };
    let mut staging: Option<ID3D11Texture2D> = None;
    unsafe {
        device.CreateTexture2D(&staging_desc, None, Some(&mut staging))?;
    }
    let staging = staging.ok_or_else(|| AppError::Other("Kunne ikke oprette staging-tekstur".into()))?;
    let staging_res: ID3D11Resource = staging.cast()?;

    let vp = D3D11_VIEWPORT {
        TopLeftX: 0.0,
        TopLeftY: 0.0,
        Width: w as f32,
        Height: h as f32,
        MinDepth: 0.0,
        MaxDepth: 1.0,
    };

    unsafe {
        context.OMSetRenderTargets(Some(&[Some(rtv)]), None);
        context.RSSetViewports(Some(&[vp]));
        context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        context.VSSetShader(&vs, None);
        context.PSSetShader(&ps, None);
        context.PSSetConstantBuffers(0, Some(&[Some(cbuf)]));
        context.Draw(3, 0);
        context.CopyResource(&staging_res, &rt_res);
    }

    let img = read_bgra_staging_to_rgb(&context, &staging_res, w, h)?;
    save_png_atomic(&img, out_path)
}

pub fn read_bgra_staging_to_rgb(
    context: &ID3D11DeviceContext,
    staging_res: &ID3D11Resource,
    w: u32,
    h: u32,
) -> Result<RgbImage, AppError> {
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        context.Map(staging_res, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
    }
    let mut img = RgbImage::new(w, h);
    let pitch = mapped.RowPitch as usize;
    let base_ptr = mapped.pData as *const u8;
    if base_ptr.is_null() || pitch < (w as usize) * 4 {
        unsafe {
            context.Unmap(staging_res, 0);
        }
        return Err(AppError::Other("Ugyldig GPU staging-buffer".into()));
    }

    for y in 0..h {
        let row = unsafe { std::slice::from_raw_parts(base_ptr.add(y as usize * pitch), w as usize * 4) };
        for x in 0..w {
            let off = x as usize * 4;
            let b = row[off];
            let g = row[off + 1];
            let r = row[off + 2];
            img.put_pixel(x, y, Rgb([r, g, b]));
        }
    }
    unsafe {
        context.Unmap(staging_res, 0);
    }
    Ok(img)
}

pub fn save_png_atomic(img: &RgbImage, out_path: &Path) -> Result<(), AppError> {
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).map_err(AppError::ConfigWrite)?;
    }
    let tmp = out_path.with_extension("png.tmp");
    img.save_with_format(&tmp, image::ImageFormat::Png)
        .map_err(|e| AppError::Other(format!("Kunne ikke gemme PNG-billede: {e}")))?;
    fs::rename(&tmp, out_path).map_err(AppError::ConfigWrite)?;
    Ok(())
}

pub fn update_constant_buffer(
    context: &ID3D11DeviceContext,
    cbuf: &ID3D11Buffer,
    uniforms: &SceneUniforms,
) -> Result<(), AppError> {
    let res: ID3D11Resource = cbuf.cast()?;
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        context.Map(&res, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
        if !mapped.pData.is_null() {
            ptr::copy_nonoverlapping(uniforms, mapped.pData as *mut SceneUniforms, 1);
        }
        context.Unmap(&res, 0);
    }
    Ok(())
}

fn create_offscreen_d3d11_device() -> Result<(ID3D11Device, ID3D11DeviceContext), AppError> {
    let levels = [D3D_FEATURE_LEVEL_11_0];
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;

    let hw = unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    };

    if hw.is_err() || device.is_none() || context.is_none() {
        device = None;
        context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }
    }

    let device = device.ok_or_else(|| AppError::Other("Kunne ikke oprette D3D11-enhed".into()))?;
    let context = context.ok_or_else(|| AppError::Other("Kunne ikke oprette D3D11-kontekst".into()))?;
    Ok((device, context))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallpaper::catalog::BUILTIN_SHADERS;

    #[test]
    fn all_eight_shaders_compile_cleanly_to_dxbc() {
        let vs = compile_vertex_shader().expect("VSMain should compile");
        assert!(!vs.is_empty());
        assert_eq!(BUILTIN_SHADERS.len(), 8);

        for spec in BUILTIN_SHADERS {
            let ps = compile_scene_pixel_shader(spec.id)
                .unwrap_or_else(|e| panic!("Shader {} failed to compile: {e}", spec.id));
            assert!(!ps.is_empty(), "Empty DXBC for {}", spec.id);
        }
    }

    #[test]
    fn offscreen_render_produces_valid_nonblack_png() {
        for spec in BUILTIN_SHADERS {
            let out = std::env::temp_dir().join(format!(
                "open_dock_shader_test_{}_{}.png",
                spec.id,
                uuid::Uuid::new_v4()
            ));
            render_shader_to_png(spec.id, 128, 72, 12.0, 1.0, &out)
                .unwrap_or_else(|e| panic!("Offscreen D3D11 render failed for {}: {e}", spec.id));
            assert!(out.exists(), "Missing PNG for {}", spec.id);

            let decoded = image::open(&out).expect("Rendered PNG should decode").to_rgb8();
            assert_eq!(decoded.dimensions(), (128, 72));

            let mut min_luma = u16::MAX;
            let mut max_luma = 0_u16;
            for px in decoded.pixels() {
                let sum = px[0] as u16 + px[1] as u16 + px[2] as u16;
                min_luma = min_luma.min(sum);
                max_luma = max_luma.max(sum);
            }
            assert!(
                max_luma > 30,
                "Rendered image {} is too dark: max_luma={max_luma}",
                spec.id
            );
            assert!(
                max_luma > min_luma,
                "Rendered image {} is uniform",
                spec.id
            );

            let _ = fs::remove_file(&out);
        }
    }
}
