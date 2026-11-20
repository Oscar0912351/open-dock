float4 PSMain(VSOut input) : SV_Target {
    float2 uv = float2(input.uv.x, 1.0 - input.uv.y);
    float aspect = u_resolution.x / max(u_resolution.y, 1.0);
    float sx = (uv.x - 0.5) * aspect;
    float t = u_time;

    // Blue-hour twilight sky
    float3 top_col = float3(0.014, 0.026, 0.052);
    float3 hor_col = float3(0.088, 0.145, 0.220);
    float3 hdr = lerp(hor_col, top_col, smoothstep(0.15, 0.95, uv.y));

    // Soft pale moonlight glow behind central peaks
    float glow = exp(-length(float2(sx - 0.15, uv.y - 0.44)) * 2.6);
    hdr += float3(0.22, 0.36, 0.54) * glow * 0.38;

    // 4 atmospheric mountain layers from far to near
    float freqs[4]  = { 1.15, 1.65, 2.35, 3.20 };
    float base_y[4] = { 0.43, 0.33, 0.23, 0.13 };
    float amps[4]   = { 0.20, 0.15, 0.11, 0.07 };
    float seeds[4]  = { 1.4,  6.2, 11.7, 19.3 };

    [unroll]
    for (int i = 0; i < 4; ++i) {
        float fi = (float)i;
        float drift = t * (0.004 + fi * 0.003);
        float ridge = base_y[i] + amps[i] * ridged_fbm(float2(sx * freqs[i] + seeds[i] + drift * 0.2, seeds[i]));
        float mask = smoothstep(ridge + 0.003, ridge - 0.003, uv.y);

        float3 layer_col = lerp(float3(0.048, 0.085, 0.140), float3(0.008, 0.015, 0.028), fi / 3.0);
        // Subtle atmospheric valley mist hugging bottom of each range
        float valley_mist = exp(-max(ridge - uv.y, 0.0) * 18.0) * 0.22;
        float mist_n = fbm4(float2(sx * (2.5 + fi) - t * (0.025 + fi * 0.012), uv.y * 8.0 + fi));
        float3 mist_col = float3(0.16, 0.26, 0.38) * (0.65 + 0.35 * mist_n);

        layer_col = lerp(layer_col, mist_col, valley_mist * (1.0 - fi * 0.18));
        hdr = lerp(hdr, layer_col, mask);

        // Drifting mist ribbon across the base
        float band_h = base_y[i] - 0.01;
        float ribbon = exp(-abs(uv.y - band_h) * 32.0) * smoothstep(0.35, 0.85, mist_n);
        hdr += float3(0.13, 0.22, 0.34) * ribbon * 0.22 * (1.0 - mask * 0.5);
    }

    return float4(finalize_color(hdr, input.pos.xy), 1.0);
}
