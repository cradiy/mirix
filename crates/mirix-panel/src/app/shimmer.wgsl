// Soft subpixel reflection, clipped to the selected item's actual rounded bounds.
fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    // The effect primitive applies its rounded clip in physical pixels.
    let x = input.uv.x + (input.uv.y - 0.5) * 0.12;
    let center = mix(-0.45, 1.45, clamp(input.time, 0.0, 1.0));
    let light = 1.0 - smoothstep(0.0, 0.35, abs(x - center));
    return vec4<f32>(1.0, 1.0, 1.0, light * 0.2);
}
