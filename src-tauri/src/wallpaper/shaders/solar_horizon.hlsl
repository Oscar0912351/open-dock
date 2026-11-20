float4 PSMain(VSOut input) : SV_Target {
    float2 uv = float2(input.uv.x, 1.0 - input.uv.y);
    float aspect = u_resolution.x / max(u_resolution.y, 1.0);
    float2 suv = float2((uv.x - 0.5) * aspect, uv.y);
    float t = u_time;

    float horizon = 0.27;

    // Sky twilight gradient
    float3 zenith = float3(0.012, 0.018, 0.036);
    float3 mid_sky = float3(0.062, 0.052, 0.072);
    float3 warm_band = float3(0.68, 0.34, 0.11);
    float3 gold_core = float3(1.15, 0.74, 0.30);

    float h_dist = abs(uv.y - horizon);
    float sun_x = 0.18 * sin(t * 0.04);
    float sun_dist = length(float2(suv.x - sun_x, (uv.y - (horizon + 0.03)) * 1.9));

    float3 sky = lerp(mid_sky, zenith, smoothstep(horizon, 0.92, uv.y));
    float band = exp(-max(uv.y - horizon, 0.0) * 6.5);
    sky += warm_band * band * 0.72;
    sky += gold_core * exp(-sun_dist * 3.4) * 0.58;

    // Slow drifting high-altitude cirrus clouds
    float cloud_n = fbm5(float2(suv.x * 1.8 - t * 0.018, (uv.y - horizon) * 6.5 + t * 0.004));
    float cloud_mask = smoothstep(0.45, 0.78, cloud_n) * smoothstep(horizon + 0.02, horizon + 0.28, uv.y) * smoothstep(0.88, 0.38, uv.y);
    float3 cloud_col = lerp(float3(0.05, 0.05, 0.07), float3(0.86, 0.52, 0.22), exp(-sun_dist * 2.2) * 0.85);
    sky = lerp(sky, cloud_col, cloud_mask * 0.55);

    float3 hdr = sky;
    if (uv.y < horizon) {
        float dy = (horizon - uv.y) / max(horizon, 0.001);
        float ripple = fbm4(float2(suv.x * 12.0, dy * 34.0 - t * 0.25)) - 0.5;
        float2 ruv = float2(uv.x + ripple * 0.014, horizon + dy * 0.75);
        float r_sun = length(float2((ruv.x - 0.5) * aspect - sun_x, (ruv.y - (horizon + 0.03)) * 1.9));
        float3 refl = lerp(mid_sky, zenith, smoothstep(horizon, 0.9, ruv.y))
            + warm_band * exp(-max(ruv.y - horizon, 0.0) * 6.5) * 0.65
            + gold_core * exp(-r_sun * 3.2) * 0.48;
        float glitter = smoothstep(0.32, 0.50, ripple + exp(-abs(suv.x - sun_x) * 6.0) * 0.38);
        refl += gold_core * glitter * 0.25 * (1.0 - dy * 0.6);
        hdr = lerp(float3(0.012, 0.014, 0.022), refl * 0.74, lerp(0.32, 0.78, pow(1.0 - dy, 1.5)));
    }

    // Crisp minimalist horizon ridge
    float ridge = horizon + 0.022 * ridged_fbm(float2(suv.x * 1.9 + 4.0, 2.0));
    float sil = smoothstep(ridge + 0.002, ridge - 0.002, uv.y) * step(horizon, uv.y);
    hdr = lerp(hdr, float3(0.008, 0.009, 0.014), sil);

    return float4(finalize_color(hdr, input.pos.xy), 1.0);
}
