cbuffer SceneUniforms : register(b0) {
    float2 u_resolution;
    float u_time;
    float u_brightness;
    float4 u_accent;
};

struct VSOut {
    float4 pos : SV_Position;
    float2 uv  : TEXCOORD0;
};

VSOut VSMain(uint id : SV_VertexID) {
    VSOut o;
    float2 pos = float2((id << 1) & 2, id & 2);
    o.pos = float4(pos * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    o.uv = pos;
    return o;
}

float hash21(float2 p) {
    p = frac(p * float2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return frac(p.x * p.y);
}

float2 hash22(float2 p) {
    float3 a = frac(float3(p.xyx) * float3(123.34, 234.34, 345.65));
    a += dot(a, a + 34.45);
    return frac(float2(a.x * a.y, a.y * a.z));
}

float vnoise(float2 p) {
    float2 i = floor(p);
    float2 f = frac(p);
    float2 u = f * f * (3.0 - 2.0 * f);
    float a = hash21(i);
    float b = hash21(i + float2(1.0, 0.0));
    float c = hash21(i + float2(0.0, 1.0));
    float d = hash21(i + float2(1.0, 1.0));
    return lerp(lerp(a, b, u.x), lerp(c, d, u.x), u.y);
}

float fbm4(float2 p) {
    float v = 0.0;
    float a = 0.5;
    float2x2 rot = float2x2(0.80, -0.60, 0.60, 0.80);
    [unroll]
    for (int i = 0; i < 4; ++i) {
        v += a * vnoise(p);
        p = mul(rot, p) * 2.02 + float2(17.1, 31.7);
        a *= 0.5;
    }
    return v;
}

float fbm5(float2 p) {
    float v = 0.0;
    float a = 0.5;
    float2x2 rot = float2x2(0.80, -0.60, 0.60, 0.80);
    [unroll]
    for (int i = 0; i < 5; ++i) {
        v += a * vnoise(p);
        p = mul(rot, p) * 2.01 + float2(13.3, 29.1);
        a *= 0.5;
    }
    return v;
}

float ridged_fbm(float2 p) {
    float v = 0.0;
    float a = 0.5;
    float2x2 rot = float2x2(0.82, -0.57, 0.57, 0.82);
    [unroll]
    for (int i = 0; i < 4; ++i) {
        float n = 1.0 - abs(vnoise(p) * 2.0 - 1.0);
        v += a * n * n;
        p = mul(rot, p) * 2.05 + float2(9.2, 23.7);
        a *= 0.5;
    }
    return v;
}

float3 aces_tonemap(float3 x) {
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    return saturate((x * (a * x + b)) / (x * (c * x + d) + e));
}

float3 finalize_color(float3 hdr, float2 frag_pos) {
    float3 mapped = aces_tonemap(hdr * u_brightness);
    float3 srgb = pow(max(mapped, 0.0), 1.0 / 2.2);
    // Triangular-PDF sub-LSB dithering eliminates 8-bit dark gradient banding.
    float r1 = hash21(frag_pos + frac(u_time * 0.13) * 19.0);
    float r2 = hash21(frag_pos * 1.37 + 47.0);
    float tri = (r1 + r2) - 1.0;
    return saturate(srgb + tri / 255.0);
}
