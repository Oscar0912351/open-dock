float4 PSMain(VSOut input) : SV_Target {
    float2 uv = float2(input.uv.x, 1.0 - input.uv.y);
    float aspect = u_resolution.x / max(u_resolution.y, 1.0);
    float t = u_time;

    // Vertical architectural fluted glass ribs
    float ribs = max(36.0 * (aspect / 1.777), 24.0);
    float flute_x = uv.x * ribs;
    float flute_phase = frac(flute_x) * 2.0 - 1.0; // -1..1 per glass reed
    float refr = flute_phase * (1.0 - flute_phase * flute_phase * 0.35) * 0.045;

    float2 r_uv = float2(uv.x + refr * 1.10, uv.y + refr * 0.22);
    float2 g_uv = float2(uv.x + refr * 0.96, uv.y + refr * 0.18);
    float2 b_uv = float2(uv.x + refr * 0.82, uv.y + refr * 0.14);

    // Slow diagonal light beams behind the glass
    float beam1_r = exp(-pow((r_uv.x * 0.8 + r_uv.y * 0.55) - (0.62 + 0.16 * sin(t * 0.11)), 2.0) * 10.5);
    float beam1_g = exp(-pow((g_uv.x * 0.8 + g_uv.y * 0.55) - (0.62 + 0.16 * sin(t * 0.11)), 2.0) * 10.5);
    float beam1_b = exp(-pow((b_uv.x * 0.8 + b_uv.y * 0.55) - (0.62 + 0.16 * sin(t * 0.11)), 2.0) * 10.5);

    float beam2_r = exp(-pow((r_uv.x * 0.65 - r_uv.y * 0.50) - (0.18 + 0.14 * cos(t * 0.09)), 2.0) * 9.0);
    float beam2_g = exp(-pow((g_uv.x * 0.65 - g_uv.y * 0.50) - (0.18 + 0.14 * cos(t * 0.09)), 2.0) * 9.0);
    float beam2_b = exp(-pow((b_uv.x * 0.65 - b_uv.y * 0.50) - (0.18 + 0.14 * cos(t * 0.09)), 2.0) * 9.0);

    float3 warm_beam = float3(0.55, 0.44, 0.30);
    float3 cool_beam = float3(0.28, 0.48, 0.68);
    float3 base = float3(0.015, 0.019, 0.026);

    float3 hdr = base;
    hdr += float3(beam1_r, beam1_g, beam1_b) * warm_beam * 0.65;
    hdr += float3(beam2_r, beam2_g, beam2_b) * cool_beam * 0.72;

    // Subtle specular sheen along each glass reed curvature
    float reed_spec = pow(1.0 - abs(flute_phase), 5.0);
    hdr += float3(0.14, 0.18, 0.23) * reed_spec * (0.25 + 0.75 * (beam1_g + beam2_g));

    // Soft glass edge shadow between reeds
    float groove = smoothstep(0.86, 1.0, abs(flute_phase));
    hdr *= 1.0 - groove * 0.34;

    return float4(finalize_color(hdr, input.pos.xy), 1.0);
}
