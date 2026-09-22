// Clip the complete candidate body, including images and native particles.
fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let half_size = input.size * 0.5;
    let radius = clamp(params.slots[0].x, 0.0, min(half_size.x, half_size.y));
    let q = abs((input.uv - vec2<f32>(0.5)) * input.size)
        - half_size + vec2<f32>(radius);
    let distance = length(max(q, vec2<f32>(0.0)))
        + min(max(q.x, q.y), 0.0) - radius;
    let coverage = 1.0 - smoothstep(-0.5, 0.5, distance);
    let color = sample_effect_image(input, input.uv);
    return vec4<f32>(color.rgb, color.a * coverage);
}
