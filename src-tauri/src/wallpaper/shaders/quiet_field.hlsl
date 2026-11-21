// Quiet Field — soft drifting colour field over a near-black base.
float4 PSMain(VSOut input) : SV_Target {
    float2 uv = float2(input.uv.x, 1.0 - input.uv.y);
    float aspect = u_resolution.x / max(u_resolution.y, 1.0);
    float2 p = float2(uv.x * aspect, uv.y);
    float t = u_time;

    // Slow domain warp so the field breathes instead of sitting still.
    float warp = fbm4(p * 1.7 + float2(t * 0.018, -t * 0.011));
    p += (warp - 0.5) * 0.30;

    // Three colour centres drifting on slow independent orbits.
    float2 c1 = float2(aspect * (0.28 + 0.10 * sin(t * 0.063)),        0.58 + 0.12 * cos(t * 0.047));
    float2 c2 = float2(aspect * (0.74 + 0.09 * cos(t * 0.052 + 2.1)),  0.34 + 0.15 * sin(t * 0.071));
    float2 c3 = float2(aspect * (0.52 + 0.13 * sin(t * 0.037 + 4.2)),  0.88 + 0.09 * cos(t * 0.061));

    float g1 = exp(-dot(p - c1, p - c1) * 5.5);
    float g2 = exp(-dot(p - c2, p - c2) * 6.5);
    float g3 = exp(-dot(p - c3, p - c3) * 8.0);

    float3 hdr = float3(0.010, 0.014, 0.022);            // near-black blue base
    hdr += float3(0.055, 0.220, 0.205) * g1;             // muted teal
    hdr += float3(0.065, 0.130, 0.320) * g2;             // steel blue
    hdr += float3(0.230, 0.160, 0.075) * g3 * 0.75;      // faint warm breath

    // Gentle vignette keeps edges quiet.
    float2 q = uv - 0.5;
    hdr *= 1.0 - dot(q, q) * 0.55;

    return float4(finalize_color(hdr, input.pos.xy), 1.0);
}
