float4 PSMain(VSOut input) : SV_Target {
    float2 uv = float2(input.uv.x, 1.0 - input.uv.y);
    float aspect = u_resolution.x / max(u_resolution.y, 1.0);
    float2 suv = float2((uv.x - 0.5) * aspect, uv.y);
    float t = u_time;

    // Deep abyssal gradient
    float3 deep = float3(0.004, 0.016, 0.026);
    float3 shallow = float3(0.022, 0.095, 0.122);
    float3 hdr = lerp(deep, shallow, pow(saturate(uv.y), 1.65));

    // Volumetric underwater sun shafts angled gently from top-left
    float angle_x = suv.x - (1.0 - uv.y) * 0.34;
    float rays = 0.0;
    [unroll]
    for (int i = 0; i < 4; ++i) {
        float fi = (float)i;
        float freq = 3.2 + fi * 1.9;
        float shift = t * (0.045 + fi * 0.018);
        float n = vnoise(float2(angle_x * freq + shift, fi * 7.3 + t * 0.03));
        rays += pow(saturate(n), 2.4) / (1.0 + fi * 0.65);
    }
    float depth_fade = smoothstep(0.04, 0.98, uv.y);
    hdr += float3(0.14, 0.52, 0.56) * rays * depth_fade * 0.42;

    // Soft surface caustic shimmer near top
    float c1 = vnoise(float2(suv.x * 6.0 + t * 0.12, suv.y * 10.0 - t * 0.08));
    float c2 = vnoise(float2(suv.x * 9.0 - t * 0.09, suv.y * 12.0 + t * 0.06));
    float caustic = pow(saturate(1.0 - abs(c1 - c2) * 3.8), 4.0);
    hdr += float3(0.32, 0.78, 0.80) * caustic * smoothstep(0.42, 1.0, uv.y) * 0.34;

    // Gentle suspended micro-particles
    float2 pgrid = float2(suv.x, uv.y - t * 0.012) * 34.0;
    float2 cell = floor(pgrid);
    float2 rnd = hash22(cell);
    float2 pf = frac(pgrid) - 0.5 - (rnd - 0.5) * 0.5;
    float mote = smoothstep(0.14, 0.0, length(pf)) * step(0.92, rnd.x) * depth_fade;
    hdr += float3(0.45, 0.84, 0.85) * mote * 0.28;

    return float4(finalize_color(hdr, input.pos.xy), 1.0);
}
