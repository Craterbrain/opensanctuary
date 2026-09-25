pub const QUAD_VERTEX_SHADER: &str = r#"
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
};

@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(model.position, 0.0, 1.0);
    out.tex_coords = model.tex_coords;
    return out;
}
"#;

pub const TRANSITION_FRAGMENT_SHADER: &str = r#"
struct Uniforms {
    progress: f32,
    transition_type: u32, // 0 = Cut, 1 = Crossfade, 2 = SlideLeft, 3 = SlideRight, 4 = WipeDown, 5 = Dissolve
    is_blackout: u32,
    is_clear: u32,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var texture_current: texture_2d<f32>;
@group(0) @binding(2) var sampler_current: sampler;
@group(0) @binding(3) var texture_next: texture_2d<f32>;
@group(0) @binding(4) var sampler_next: sampler;

@fragment
fn fs_main(@location(0) tex_coords: vec2<f32>) -> @location(0) vec4<f32> {
    if (uniforms.is_blackout == 1u) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    var uv = tex_coords;
    var color_curr = textureSample(texture_current, sampler_current, uv);
    var color_next = textureSample(texture_next, sampler_next, uv);
    var p = clamp(uniforms.progress, 0.0, 1.0);

    var final_color = color_curr;

    switch (uniforms.transition_type) {
        case 0u: { // Cut
            if (p >= 1.0) {
                final_color = color_next;
            } else {
                final_color = color_curr;
            }
        }
        case 1u: { // Crossfade
            final_color = mix(color_curr, color_next, p);
        }
        case 2u: { // SlideLeft
            var offset = vec2<f32>(p, 0.0);
            if (uv.x < 1.0 - p) {
                final_color = textureSample(texture_current, sampler_current, uv + offset);
            } else {
                final_color = textureSample(texture_next, sampler_next, uv - vec2<f32>(1.0 - p, 0.0));
            }
        }
        case 3u: { // SlideRight
            var offset = vec2<f32>(p, 0.0);
            if (uv.x > p) {
                final_color = textureSample(texture_current, sampler_current, uv - offset);
            } else {
                final_color = textureSample(texture_next, sampler_next, uv + vec2<f32>(1.0 - p, 0.0));
            }
        }
        case 4u: { // WipeDown
            if (uv.y <= p) {
                final_color = color_next;
            } else {
                final_color = color_curr;
            }
        }
        case 5u: { // Dissolve (Luma threshold)
            var luma = dot(color_next.rgb, vec3<f32>(0.299, 0.587, 0.114));
            if (luma <= p) {
                final_color = color_next;
            } else {
                final_color = mix(color_curr, color_next, p * 0.5);
            }
        }
        default: {
            final_color = mix(color_curr, color_next, p);
        }
    }

    return final_color;
}
"#;
