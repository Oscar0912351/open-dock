float silk_height(float2 p, float t) {
    float2 q = float2(
        fbm4(p * 1.15 + float2(t * 0.055, -t * 0.032)),
        fbm4(p * 1.15 + float2(5.2 - t * 0.041, 1.3 + t * 0.048))
    );
    float2 r = float2(
        fbm4(p * 1.55 + 1.8 * q + float2(1.7, 9.2) + t * 0.025),
        fbm4(p * 1.55 + 1.8 * q + float2(8.3, 2.8) - t * 0.020)
    );
    float fold = sin(p.x * 2.1 + r.x * 4.5 + p.y * 0.8);
    return 0.55 * fbm4(p + 1.4 * r) + 0.25 * (0.5 + 0.5 * fold);
}

float4 PSMain(VSOut input) : SV_Target {
    float2 uv = float2(input.uv.x, 1.0 - input.uv.y);
    float aspect = u_resolution.x / max(u_resolution.y, 1.0);
    float2 p = float2((uv.x - 0.5) * aspect, uv.y - 0.5) * 1.9;
    float t = u_time;

    float e = 0.004;
    float h  = silk_height(p, t);
    float hx = silk_height(p + float2(e, 0.0), t);
    float hy = silk_height(p + float2(0.0, e), t);

    float3 n = normalize(float3((h - hx) / e * 0.34, (h - hy) / e * 0.34, 1.0));
    float3 v = float3(0.0, 0.0, 1.0);
    float3 l1 = normalize(float3(-0.55, 0.65, 0.52));
    float3 l2 = normalize(float3(0.75, -0.35, 0.45));

    float diff1 = max(dot(n, l1), 0.0);
    float diff2 = max(dot(n, l2), 0.0);
    float3 half1 = normalize(l1 + v);
    float spec1 = pow(max(dot(n, half1), 0.0), 36.0);
    float fresnel = pow(1.0 - max(dot(n, v), 0.0), 3.4);

    float3 base = float3(0.012, 0.014, 0.018);
    float3 slate = float3(0.055, 0.068, 0.088);
    float3 silver = float3(0.62, 0.72, 0.84);
    float3 warm_rim = float3(0.28, 0.34, 0.42);

    float3 hdr = base;
    hdr += slate * (diff1 * 0.85 + h * 0.25);
    hdr += warm_rim * diff2 * 0.14;
    hdr += silver * (spec1 * 0.42 + fresnel * 0.26 * (0.4 + diff1));

    // Gentle vignette for crisp edge contrast
    float vig = 1.0 - dot(uv - 0.5, uv - 0.5) * 0.42;
    hdr *= vig;

    return float4(finalize_color(hdr, input.pos.xy), 1.0);
}
