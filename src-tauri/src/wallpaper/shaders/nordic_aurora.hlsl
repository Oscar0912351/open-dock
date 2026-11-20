float3 sample_aurora_sky(float2 uv, float t) {
    float3 sky_bot = float3(0.012, 0.032, 0.048);
    float3 sky_top = float3(0.004, 0.009, 0.018);
    float3 col = lerp(sky_bot, sky_top, smoothstep(0.28, 0.96, uv.y));

    // Sparse crisp stars above horizon
    float2 grid = uv * float2(130.0, 74.0);
    float2 cell = floor(grid);
    float2 f = frac(grid) - 0.5;
    float2 rnd = hash22(cell);
    float star_mask = step(0.965, rnd.x) * smoothstep(0.35, 0.75, uv.y);
    float d = length(f - (rnd - 0.5) * 0.55);
    float twinkle = 0.65 + 0.35 * sin(t * (1.2 + rnd.y * 2.0) + rnd.x * 60.0);
    col += float3(0.75, 0.86, 1.0) * smoothstep(0.18, 0.0, d) * star_mask * twinkle * 0.45;

    // Volumetric layered aurora curtains
    float3 aur = float3(0.0, 0.0, 0.0);
    [unroll]
    for (int i = 0; i < 6; ++i) {
        float fi = (float)i;
        float depth = 0.25 + fi * 0.14;
        float2 p = float2(uv.x * (1.7 + fi * 0.22) + t * 0.028 * (1.0 - fi * 0.08),
                          uv.y * 2.1 - fi * 0.18);
        float curtain_wave = sin(p.x * 2.2 + fbm4(float2(p.x * 0.85, t * 0.06 + fi)) * 4.2);
        float ribbon = smoothstep(0.38, 0.0, abs(uv.y - (0.58 + 0.12 * curtain_wave + fi * 0.025)));
        float rays = fbm4(float2(p.x * 5.2 + curtain_wave * 0.7, t * 0.10 + fi * 3.1));
        float vertical_fade = smoothstep(0.36, 0.56, uv.y) * smoothstep(0.95, 0.62, uv.y);
        float intensity = ribbon * smoothstep(0.25, 0.85, rays) * vertical_fade / (1.0 + depth * 1.4);

        float3 mint = float3(0.10, 0.92, 0.66);
        float3 ice  = float3(0.14, 0.58, 0.88);
        float3 tint = lerp(mint, ice, smoothstep(0.55, 0.82, uv.y + fi * 0.03));
        aur += tint * intensity * 0.95;
    }

    return col + aur;
}

float4 PSMain(VSOut input) : SV_Target {
    float2 uv = float2(input.uv.x, 1.0 - input.uv.y);
    float aspect = u_resolution.x / max(u_resolution.y, 1.0);
    float2 suv = float2((uv.x - 0.5) * aspect + 0.5, uv.y);
    float t = u_time;

    float horizon = 0.30;
    float ridge_far = horizon + 0.11 * ridged_fbm(float2(suv.x * 1.55 + 2.3, 1.7));
    float ridge_near = horizon + 0.075 * ridged_fbm(float2(suv.x * 2.4 + 7.1, 4.4));

    float3 sky = sample_aurora_sky(uv, t);
    float3 hdr = sky;

    if (uv.y < horizon) {
        // Still fjord water reflection with subtle wind ripples
        float dy = (horizon - uv.y) / max(horizon, 0.001);
        float2 ruv = float2(uv.x, horizon + dy * 0.72);
        float ripple = (fbm4(float2(suv.x * 14.0, dy * 38.0 - t * 0.35)) - 0.5) * 0.018 * (0.3 + dy);
        ruv.x += ripple;
        ruv.y += ripple * 0.6;
        float3 refl = sample_aurora_sky(ruv, t);
        float fresnel = lerp(0.28, 0.72, pow(1.0 - dy, 1.8));
        float3 water_base = float3(0.006, 0.018, 0.026);
        hdr = lerp(water_base, refl * 0.72, fresnel);
    }

    // Crisp mountain silhouette above water
    float m_far = smoothstep(ridge_far + 0.003, ridge_far - 0.003, uv.y) * step(horizon, uv.y);
    float m_near = smoothstep(ridge_near + 0.0025, ridge_near - 0.0025, uv.y) * step(horizon, uv.y);
    hdr = lerp(hdr, float3(0.010, 0.020, 0.028), m_far * 0.90);
    hdr = lerp(hdr, float3(0.005, 0.010, 0.015), m_near);

    // Subtle shoreline mist
    float shore = exp(-abs(uv.y - horizon) * 65.0) * 0.06;
    hdr += float3(0.18, 0.45, 0.48) * shore;

    return float4(finalize_color(hdr, input.pos.xy), 1.0);
}
