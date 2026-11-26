use std::{mem::size_of, ptr};

use windows::{
    core::Interface,
    Win32::{
        Devices::Display::{
            DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
            DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
            DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
            DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
            DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO,
            DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SDR_WHITE_LEVEL,
            DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
        },
        Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS},
        Graphics::{
            Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            Direct3D11::{
                ID3D11Buffer, ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader,
                ID3D11RasterizerState, ID3D11RenderTargetView, ID3D11Resource,
                ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
                D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_SHADER_RESOURCE, D3D11_BUFFER_DESC,
                D3D11_CPU_ACCESS_WRITE, D3D11_CULL_NONE, D3D11_FILL_SOLID,
                D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD, D3D11_RASTERIZER_DESC,
                D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_DYNAMIC, D3D11_VIEWPORT,
            },
            Dxgi::{
                Common::{
                    DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709,
                    DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, DXGI_FORMAT, DXGI_SAMPLE_DESC,
                },
                IDXGIOutput1, IDXGIOutput6,
            },
        },
    },
};

use crate::wallpaper::shader_compiler::compile_hlsl;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct MonitorColorInfo {
    pub(super) hdr_enabled: bool,
    pub(super) sdr_white_scale: f32,
}

fn utf16_z_slice(buf: &[u16]) -> &[u16] {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    &buf[..end]
}

pub(super) fn device_name_display(buf: &[u16]) -> String {
    String::from_utf16_lossy(utf16_z_slice(buf))
}

pub fn sdr_white_scale_from_raw(raw: u32) -> f32 {
    if raw == 0 {
        1.0
    } else {
        (raw as f32 / 1000.0).clamp(1.0, 20.0)
    }
}

pub(super) fn query_monitor_color_info(
    device_name: &[u16; 32],
    dxgi_output: &IDXGIOutput1,
) -> MonitorColorInfo {
    let dxgi_hdr = dxgi_output
        .cast::<IDXGIOutput6>()
        .ok()
        .and_then(|out6| unsafe { out6.GetDesc1() }.ok())
        .map(|d1| {
            matches!(
                d1.ColorSpace,
                DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020
                    | DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709
            )
        })
        .unwrap_or(false);

    let target_gdi = utf16_z_slice(device_name);
    let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> = Vec::new();
    let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = Vec::new();
    let mut queried = false;

    for _ in 0..3 {
        let mut path_count = 0_u32;
        let mut mode_count = 0_u32;
        let sz_rc = unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
        };
        if sz_rc != ERROR_SUCCESS || path_count == 0 {
            break;
        }
        paths.resize(path_count as usize, DISPLAYCONFIG_PATH_INFO::default());
        modes.resize(mode_count as usize, DISPLAYCONFIG_MODE_INFO::default());
        let q_rc = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                None,
            )
        };
        if q_rc == ERROR_SUCCESS {
            paths.truncate(path_count as usize);
            queried = true;
            break;
        }
        if q_rc != ERROR_INSUFFICIENT_BUFFER {
            break;
        }
    }

    if queried && !target_gdi.is_empty() {
        for path in &paths {
            let mut sn = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: path.sourceInfo.adapterId,
                    id: path.sourceInfo.id,
                },
                ..Default::default()
            };
            let rc_sn = unsafe { DisplayConfigGetDeviceInfo(&mut sn.header) };
            if rc_sn != ERROR_SUCCESS.0 as i32 {
                continue;
            }
            if utf16_z_slice(&sn.viewGdiDeviceName) != target_gdi {
                continue;
            }

            let mut ac = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
                    size: size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            let ccd_hdr = if unsafe { DisplayConfigGetDeviceInfo(&mut ac.header) }
                == ERROR_SUCCESS.0 as i32
            {
                (unsafe { ac.Anonymous.value } & 0x2) != 0
            } else {
                false
            };

            let mut wl = DISPLAYCONFIG_SDR_WHITE_LEVEL {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
                    size: size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            let sdr_white_scale = if unsafe { DisplayConfigGetDeviceInfo(&mut wl.header) }
                == ERROR_SUCCESS.0 as i32
            {
                sdr_white_scale_from_raw(wl.SDRWhiteLevel)
            } else {
                1.0
            };

            return MonitorColorInfo {
                hdr_enabled: ccd_hdr || dxgi_hdr,
                sdr_white_scale,
            };
        }
    }

    MonitorColorInfo {
        hdr_enabled: dxgi_hdr,
        sdr_white_scale: 1.0,
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct HdrToSdrParams {
    inv_sdr_scale: f32,
    _pad: [f32; 3],
}

const HDR_TO_SDR_HLSL: &str = r#"
Texture2D<float4> srcTex : register(t0);

cbuffer Params : register(b0) {
    float invSdrScale;
    float _pad0;
    float _pad1;
    float _pad2;
};

float4 VSMain(uint id : SV_VertexID) : SV_Position {
    float2 uv = float2((id << 1) & 2, id & 2);
    return float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float linearToSrgb(float c) {
    c = saturate(c);
    return (c <= 0.0031308) ? (c * 12.92) : (1.055 * pow(max(c, 0.0031308), 1.0 / 2.4) - 0.055);
}

float4 PSMain(float4 pos : SV_Position) : SV_Target {
    float3 lin = srcTex.Load(int3(int2(pos.xy), 0)).rgb * invSdrScale;
    return float4(linearToSrgb(lin.r), linearToSrgb(lin.g), linearToSrgb(lin.b), 1.0);
}
"#;

pub(super) struct HdrToSdrConverter {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    cbuf: ID3D11Buffer,
    raster_state: ID3D11RasterizerState,
}

impl HdrToSdrConverter {
    pub(super) fn new(device: &ID3D11Device) -> Result<Self, String> {
        let vs_bytes = compile_hlsl(
            HDR_TO_SDR_HLSL,
            b"VSMain\0",
            b"vs_5_0\0",
            b"hdr_to_sdr.hlsl\0",
        )
        .map_err(|e| format!("HDR VS kompilering: {e:?}"))?;
        let ps_bytes = compile_hlsl(
            HDR_TO_SDR_HLSL,
            b"PSMain\0",
            b"ps_5_0\0",
            b"hdr_to_sdr.hlsl\0",
        )
        .map_err(|e| format!("HDR PS kompilering: {e:?}"))?;

        let mut vs: Option<ID3D11VertexShader> = None;
        let mut ps: Option<ID3D11PixelShader> = None;
        unsafe {
            device
                .CreateVertexShader(&vs_bytes, None, Some(&mut vs))
                .map_err(|e| format!("HDR CreateVertexShader: {e}"))?;
            device
                .CreatePixelShader(&ps_bytes, None, Some(&mut ps))
                .map_err(|e| format!("HDR CreatePixelShader: {e}"))?;
        }
        let vs = vs.ok_or_else(|| "HDR vertex shader returnerede None".to_string())?;
        let ps = ps.ok_or_else(|| "HDR pixel shader returnerede None".to_string())?;

        let cbuf_desc = D3D11_BUFFER_DESC {
            ByteWidth: size_of::<HdrToSdrParams>() as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let mut cbuf: Option<ID3D11Buffer> = None;
        unsafe {
            device
                .CreateBuffer(&cbuf_desc, None, Some(&mut cbuf))
                .map_err(|e| format!("HDR CreateBuffer: {e}"))?;
        }
        let cbuf = cbuf.ok_or_else(|| "HDR cbuf returnerede None".to_string())?;

        let raster_desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            DepthClipEnable: false.into(),
            ScissorEnable: false.into(),
            ..Default::default()
        };
        let mut raster_state: Option<ID3D11RasterizerState> = None;
        unsafe {
            device
                .CreateRasterizerState(&raster_desc, Some(&mut raster_state))
                .map_err(|e| format!("HDR CreateRasterizerState: {e}"))?;
        }
        let raster_state =
            raster_state.ok_or_else(|| "HDR raster_state returnerede None".to_string())?;

        Ok(Self {
            vs,
            ps,
            cbuf,
            raster_state,
        })
    }

    pub(super) fn convert(
        &self,
        ctx: &ID3D11DeviceContext,
        srv: &ID3D11ShaderResourceView,
        rtv: &ID3D11RenderTargetView,
        width: u32,
        height: u32,
        inv_sdr_scale: f32,
    ) -> Result<(), String> {
        let params = HdrToSdrParams {
            inv_sdr_scale,
            _pad: [0.0; 3],
        };
        let cbuf_res: ID3D11Resource = self
            .cbuf
            .cast()
            .map_err(|e| format!("HDR cbuf ID3D11Resource cast: {e}"))?;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            ctx.Map(&cbuf_res, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
                .map_err(|e| format!("HDR cbuf Map: {e}"))?;
            if !mapped.pData.is_null() {
                ptr::copy_nonoverlapping(&params, mapped.pData as *mut HdrToSdrParams, 1);
            }
            ctx.Unmap(&cbuf_res, 0);
        }

        let vp = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: width.max(1) as f32,
            Height: height.max(1) as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };

        unsafe {
            // Direct2D deler immediate context med D3D11 og efterlader egen pipeline-state.
            ctx.IASetInputLayout(None);
            ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.VSSetShader(&self.vs, None);
            ctx.GSSetShader(None, None);
            ctx.HSSetShader(None, None);
            ctx.DSSetShader(None, None);
            ctx.PSSetShader(&self.ps, None);
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.cbuf.clone())]));
            ctx.PSSetShaderResources(0, Some(&[Some(srv.clone())]));
            ctx.RSSetState(&self.raster_state);
            ctx.RSSetViewports(Some(&[vp]));
            ctx.OMSetBlendState(None, None, 0xFFFF_FFFF);
            ctx.OMSetDepthStencilState(None, 0);
            ctx.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
            ctx.Draw(3, 0);
            ctx.OMSetRenderTargets(None, None);
            ctx.PSSetShaderResources(0, Some(&[None]));
        }

        Ok(())
    }
}

pub(super) struct HdrInputCache {
    pub(super) tex: ID3D11Texture2D,
    pub(super) srv: ID3D11ShaderResourceView,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) format: DXGI_FORMAT,
}

pub(super) fn ensure_hdr_input_cache<'a>(
    d3d_device: &ID3D11Device,
    slot: &'a mut Option<HdrInputCache>,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
) -> Result<&'a HdrInputCache, String> {
    let matches = slot
        .as_ref()
        .map(|c| c.width == width && c.height == height && c.format == format)
        .unwrap_or(false);
    if !matches {
        let tex_desc = D3D11_TEXTURE2D_DESC {
            Width: width.max(1),
            Height: height.max(1),
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut tex: Option<ID3D11Texture2D> = None;
        unsafe {
            d3d_device
                .CreateTexture2D(&tex_desc, None, Some(&mut tex))
                .map_err(|e| format!("CreateTexture2D HDR cache: {e}"))?;
        }
        let tex = tex.ok_or_else(|| "CreateTexture2D HDR cache returnerede None".to_string())?;
        let res: ID3D11Resource = tex
            .cast()
            .map_err(|e| format!("HDR cache ID3D11Resource cast: {e}"))?;
        let mut srv: Option<ID3D11ShaderResourceView> = None;
        unsafe {
            d3d_device
                .CreateShaderResourceView(&res, None, Some(&mut srv))
                .map_err(|e| format!("CreateShaderResourceView HDR cache: {e}"))?;
        }
        let srv = srv.ok_or_else(|| "CreateShaderResourceView returnerede None".to_string())?;
        *slot = Some(HdrInputCache {
            tex,
            srv,
            width,
            height,
            format,
        });
    }
    slot.as_ref()
        .ok_or_else(|| "HDR input cache ikke initialiseret".to_string())
}
