float4 PSMain(VSOut input) : SV_Target {
    float2 uv = float2(input.uv.x, 1.0 - input.uv.y);
    float aspect = u_resolution.x / max(u_resolution.y, 1.0);
    float sx = (uv.x - 0.5) * aspect;
    float t = u_time;

    // Warm ceramic-to-amber sky
    float3 sky_top = float3(0.56, 0.64, 0.72);
    float3 sky_hor = float3(0.96, 0.82, 0.62);
    float3 hdr = lerp(sky_hor, sky_top, pow(saturate((uv.y - 0.34) / 0.66), 0.75));

    // Soft drifting high-altitude haze veils in the morning sky
    float sky_veil = fbm4(float2(sx * 1.35 - t * 0.045, uv.y * 4.2 + t * 0.008));
    float veil_mask = smoothstep(0.42, 0.82, sky_veil)
        * smoothstep(0.42, 0.72, uv.y)
        * smoothstep(1.0, 0.70, uv.y);
    hdr = lerp(hdr, float3(0.98, 0.89, 0.74), veil_mask * 0.24);

    // Soft warm morning sun haze with gentle atmospheric pulse
    float sun_pulse = 0.22 + 0.025 * sin(t * 0.45);
    float sun_glow = exp(-length(float2(sx - 0.28 - 0.02 * sin(t * 0.14), uv.y - 0.64)) * 2.2);
    hdr += float3(1.0, 0.84, 0.55) * sun_glow * sun_pulse;

    // 4 sculpted layered sand dunes with parallax drift and sunlit crests
    float crests[4]  = { 0.54, 0.43, 0.31, 0.19 };
    float amps[4]    = { 0.075, 0.068, 0.055, 0.045 };
    float freqs[4]   = { 1.35, 1.90, 2.55, 3.10 };
    float phases[4]  = { 0.6, 2.8, 4.9, 1.5 };

    [unroll]
    for (int i = 0; i < 4; ++i) {
        float fi = (float)i;
        float drift = t * (0.065 + fi * 0.042);
        float px = sx * freqs[i] + phases[i] + drift;
        float w = sin(px) * 0.55
            + (fbm4(float2(px * 0.65, fi * 3.1 + t * 0.014)) - 0.5) * 0.90;
        float ridge = crests[i] + amps[i] * w;
        float mask = smoothstep(ridge + 0.003, ridge - 0.003, uv.y);

        // Slope shading across dune crest
        float slope = cos(px);
        float3 lit_sand = lerp(float3(0.84, 0.62, 0.38), float3(0.68, 0.43, 0.23), fi / 3.0);
        float3 shadow_sand = lerp(float3(0.56, 0.38, 0.24), float3(0.36, 0.21, 0.12), fi / 3.0);
        float lit_fac = smoothstep(-0.35, 0.55, slope);
        float3 dune_col = lerp(shadow_sand, lit_sand, lit_fac);

        // Subtle wind-swept sand shimmer across the sunlit face
        float wind_tex = fbm4(float2(sx * (4.2 + fi * 1.4) - t * (0.22 + fi * 0.08), uv.y * 16.0 + fi));
        dune_col += float3(0.95, 0.78, 0.52) * smoothstep(0.48, 0.85, wind_tex) * 0.07 * lit_fac;

        // Delicate rim highlight at the dune ridge
        float crest_glow = exp(-max(ridge - uv.y, 0.0) * 55.0) * 0.18 * (0.5 + 0.5 * lit_fac);
        dune_col += float3(0.95, 0.76, 0.48) * crest_glow;

        // Warm atmospheric depth haze + drifting sand veil over ridges
        float dust_wave = 0.65 + 0.35 * fbm4(float2(sx * 2.4 - t * (0.18 + fi * 0.06), fi * 5.3));
        float depth_haze = exp(-abs(uv.y - ridge) * 14.0) * 0.10 * dust_wave * (1.0 - fi / 3.5);
        dune_col = lerp(dune_col, sky_hor, depth_haze);

        hdr = lerp(hdr, dune_col, mask);
    }

    return float4(finalize_color(hdr, input.pos.xy), 1.0);
}
