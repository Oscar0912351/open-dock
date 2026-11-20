float topo_height(float2 p, float t) {
    float2 q = p * 0.95 + float2(t * 0.028, -t * 0.018);
    float w1 = fbm4(q);
    float w2 = fbm4(q * 1.45 + float2(w1 * 1.3, -w1 * 0.9));
    return w1 * 0.55 + w2 * 0.45;
}

float4 PSMain(VSOut input) : SV_Target {
    float2 uv = float2(input.uv.x, 1.0 - input.uv.y);
    float aspect = u_resolution.x / max(u_resolution.y, 1.0);
    float2 p = float2((uv.x - 0.5) * aspect, uv.y - 0.5) * 2.4;
    float t = u_time;

    float e = 0.005;
    float h  = topo_height(p, t);
    float hx = topo_height(p + float2(e, 0.0), t);
    float hy = topo_height(p + float2(0.0, e), t);

    float3 n = normalize(float3((h - hx) / e * 0.28, (h - hy) / e * 0.28, 1.0));
    float3 l = normalize(float3(-0.45, 0.72, 0.52));
    float3 v = float3(0.0, 0.0, 1.0);

    float diff = max(dot(n, l), 0.0);
    float spec = pow(max(dot(n, normalize(l + v)), 0.0), 28.0);

    // Matte obsidian-graphite surface
    float3 graphite = float3(0.014, 0.016, 0.020) + float3(0.040, 0.044, 0.052) * diff + float3(0.09, 0.10, 0.12) * spec;

    // Anti-aliased topographic contour lines
    float levels = h * 26.0;
    float f = abs(frac(levels - 0.5) - 0.5) / max(fwidth(levels), 0.0001);
    float line_mask = 1.0 - smoothstep(0.0, 1.35, f);
    float major_line = 1.0 - smoothstep(0.0, 1.65, abs(frac(h * 6.5 - 0.5) - 0.5) / max(fwidth(h * 6.5), 0.0001));

    float3 hdr = graphite;
    hdr += float3(0.16, 0.19, 0.23) * line_mask * 0.36;
    hdr += float3(0.32, 0.38, 0.45) * major_line * 0.40;

    // Quiet vignette
    float vig = 1.0 - dot(uv - 0.5, uv - 0.5) * 0.48;
    hdr *= vig;

    return float4(finalize_color(hdr, input.pos.xy), 1.0);
}
