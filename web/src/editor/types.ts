/**
 * Core types for OpenSanctuary Canvas Slide Editor.
 * Invariant: inv.element.transform-normalized (x, y, w, h in 0.0..=1.0)
 */

// NOTE: ElementTransform / TextBlock / SlideElement below are hand-mirrored by
// src/core/models.rs (same names) — there's no shared schema or codegen between
// them. If you add/rename/remove a field here, make the matching edit there too,
// or the two sides will silently drift (wire JSON that (de)serializes fine on
// one side but is missing/misread on the other).

export interface ElementTransform {
  x: number; // 0.0 .. 1.0 (relative to 16:9 canvas width)
  y: number; // 0.0 .. 1.0 (relative to 16:9 canvas height)
  w: number; // 0.0 .. 1.0
  h: number; // 0.0 .. 1.0
  rotation_deg: number; // 0.0 .. 360.0
  z_index: number;
  locked: boolean;
  opacity: number; // 0.0 .. 1.0
}

export function defaultTransform(): ElementTransform {
  return {
    x: 0.05,
    y: 0.07,
    w: 0.90,
    h: 0.86,
    rotation_deg: 0,
    z_index: 0,
    locked: false,
    opacity: 1.0
  };
}

export function normalizeTransform(t: ElementTransform): ElementTransform {
  return {
    ...t,
    x: Math.max(0, Math.min(1, t.x)),
    y: Math.max(0, Math.min(1, t.y)),
    w: Math.max(0.01, Math.min(1, t.w)),
    h: Math.max(0.01, Math.min(1, t.h)),
    opacity: Math.max(0, Math.min(1, t.opacity)),
    rotation_deg: ((t.rotation_deg % 360) + 360) % 360
  };
}

export interface TextRun {
  text: string;
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
  strike?: boolean;
  color?: string;
  font_family?: string;
  font_size_pt?: number;
  letter_spacing_px?: number;
  baseline_shift?: string;
}

export interface TextParagraphStyle {
  align?: 'left' | 'center' | 'right' | 'justify';
  line_height?: number;
  space_before_pt?: number;
  space_after_pt?: number;
  bullet_kind?: 'none' | 'disc' | 'decimal' | 'liturgical';
  indent_level?: number;
}

export interface TextOutline {
  width: number;
  color: string;
}

export interface ElementShadow {
  dx: number;
  dy: number;
  blur: number;
  color: string;
}

export interface ElementReflection {
  enabled: boolean;
  opacity: number;
}

export interface ElementGlow {
  radius: number;
  color: string;
}

export interface ElementEffects {
  outline?: TextOutline;
  shadow?: ElementShadow;
  reflection?: ElementReflection;
  glow?: ElementGlow;
  blend_mode?: string;
}

export interface TextBlock {
  runs: TextRun[];
  paragraph_style?: TextParagraphStyle;
  effects?: ElementEffects;
  autofit?: boolean;
}

export interface CropRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export type SlideElement =
  | {
      type: 'TextBlock';
      id: string;
      transform: ElementTransform;
      block: TextBlock;
    }
  | {
      type: 'Image';
      id: string;
      transform: ElementTransform;
      file_path: string;
      crop?: CropRect;
      mask_shape?: string;
      alt_text?: string;
    }
  | {
      type: 'Video';
      id: string;
      transform: ElementTransform;
      file_path: string;
      in_point_s?: number;
      out_point_s?: number;
      loop_playback?: boolean;
      is_muted?: boolean;
      volume?: number;
    }
  | {
      type: 'Shape';
      id: string;
      transform: ElementTransform;
      shape_kind: string;
      fill_color: string;
      stroke_color?: string;
      stroke_width?: number;
    }
  | {
      type: 'Line';
      id: string;
      transform: ElementTransform;
      line_kind: string;
      color: string;
      stroke_width?: number;
      start_arrow?: boolean;
      end_arrow?: boolean;
    }
  | {
      type: 'Table';
      id: string;
      transform: ElementTransform;
      rows: number;
      cols: number;
      cells: string[][];
    }
  | {
      type: 'Group';
      id: string;
      transform: ElementTransform;
      children: SlideElement[];
    };

export type SlideBackground =
  | { kind: 'Solid'; data: string }
  | {
      kind: 'Gradient';
      data: {
        kind: 'linear' | 'radial';
        stops: Array<{ offset: number; color: string }>;
        angle_deg?: number;
      };
    }
  | {
      kind: 'Image';
      data: {
        file_path: string;
        opacity?: number;
      };
    }
  | {
      kind: 'Video';
      data: {
        file_path: string;
        loop_playback?: boolean;
        is_muted?: boolean;
      };
    }
  | {
      kind: 'Pattern';
      data: {
        pattern_name: string;
        foreground: string;
        background: string;
      };
    };

export interface SlideTransition {
  kind: string;
  duration_ms: number;
}

export interface CcliMetadata {
  title?: string;
  author?: string;
  copyright?: string;
  ccli_number?: string;
}

export interface EditorSlide {
  id: string;
  text: string;
  header?: string;
  label?: string;
  tag?: string;
  notes?: string;
  background?: string;
  elements: SlideElement[];
  speaker_notes?: string;
  ccli_metadata?: CcliMetadata;
  background_v2?: SlideBackground;
  transition?: SlideTransition;
  slide_document_version?: number;
  duration_seconds?: number | null;
  /** Scripture verse reference (e.g. "John 3:16"), kept separate from `text` so a
   * Scripture theme's reference_position can render it inline/top/bottom. */
  reference_label?: string;
}

/**
 * One edit within a `BatchSlideEdit` (see `EditorHistoryManager.computeBatchOps`
 * below, the only real producer of these on the wire). Mirrors the Rust
 * `#[serde(tag = "op_type", content = "payload")]` enum of the same name in
 * src/core/commands.rs field-for-field -- an "adjacently tagged" enum
 * produces exactly `{op_type: "...", payload: {...}}`, same as this type
 * already needs to construct by hand, so this is a type-safety hardening
 * with no wire-format change: a typo'd field name or wrong payload shape is
 * now a compile error here instead of a silent `tracing::warn!`-and-drop on
 * the Rust side.
 */
export type SlideEditOp =
  | { op_type: 'AddElement'; payload: SlideElement }
  | { op_type: 'RemoveElement'; payload: { element_id: string } }
  | { op_type: 'UpdateTransform'; payload: { element_id: string; transform: ElementTransform } }
  | { op_type: 'UpdateTextBlockContent'; payload: { element_id: string; runs: TextRun[]; paragraph_style?: TextParagraphStyle } }
  | { op_type: 'UpdateElementEffects'; payload: { element_id: string; effects: ElementEffects } }
  | { op_type: 'ReorderElements'; payload: { element_id: string; to_z: number } }
  | { op_type: 'GroupElements'; payload: { element_ids: string[] } }
  | { op_type: 'UngroupElements'; payload: { group_id: string } }
  | { op_type: 'SetSpeakerNotes'; payload: { notes: string } }
  | { op_type: 'SetCcliMetadata'; payload: { metadata: CcliMetadata } }
  | { op_type: 'SetBackground'; payload: { background: SlideBackground } }
  | { op_type: 'SetTransition'; payload: { transition: SlideTransition } };

export interface TextStyleUpdate {
  font_family?: string;
  font_size_pt?: number;
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
  strike?: boolean;
  baseline_shift?: 'normal' | 'sub' | 'super';
  color?: string;
  align?: 'left' | 'center' | 'right' | 'justify';
  line_height?: number;
  bullet_kind?: 'none' | 'disc' | 'decimal' | 'liturgical';
  indent_level?: number;
  indent_delta?: number;
  autofit?: boolean;
  outline_width?: number;
  outline_color?: string;
  shadow_blur?: number;
  shadow_color?: string;
  shadow_dx?: number;
  shadow_dy?: number;
}

export interface BackgroundPreset {
  name: string;
  background: SlideBackground;
  preview: string;
}

export const SOLID_PALETTE_PRESETS = [
  { name: 'Pure Black', color: '#000000' },
  { name: 'Charcoal', color: '#18181b' },
  { name: 'Deep Navy', color: '#0f172a' },
  { name: 'Midnight Blue', color: '#1e1b4b' },
  { name: 'Royal Purple', color: '#3b0764' },
  { name: 'Dark Slate', color: '#0f2b36' },
  { name: 'Deep Crimson', color: '#450a0a' },
  { name: 'Forest Green', color: '#052e16' },
  { name: 'Rich Maroon', color: '#581c87' },
  { name: 'Dark Teal', color: '#134e4a' }
];

export const GRADIENT_PRESETS: BackgroundPreset[] = [
  {
    name: 'Midnight Ocean',
    background: {
      kind: 'Gradient',
      data: {
        kind: 'linear',
        angle_deg: 180,
        stops: [
          { offset: 0, color: '#0f2027' },
          { offset: 0.5, color: '#203a43' },
          { offset: 1, color: '#2c5364' }
        ]
      }
    },
    preview: 'linear-gradient(180deg, #0f2027, #203a43, #2c5364)'
  },
  {
    name: 'Royal Velvet',
    background: {
      kind: 'Gradient',
      data: {
        kind: 'linear',
        angle_deg: 135,
        stops: [
          { offset: 0, color: '#1e1b4b' },
          { offset: 0.5, color: '#3b0764' },
          { offset: 1, color: '#4c0519' }
        ]
      }
    },
    preview: 'linear-gradient(135deg, #1e1b4b, #3b0764, #4c0519)'
  },
  {
    name: 'Deep Amethyst',
    background: {
      kind: 'Gradient',
      data: {
        kind: 'linear',
        angle_deg: 180,
        stops: [
          { offset: 0, color: '#18052e' },
          { offset: 0.5, color: '#3a105c' },
          { offset: 1, color: '#1f0b35' }
        ]
      }
    },
    preview: 'linear-gradient(180deg, #18052e, #3a105c, #1f0b35)'
  },
  {
    name: 'Charcoal Fade',
    background: {
      kind: 'Gradient',
      data: {
        kind: 'linear',
        angle_deg: 180,
        stops: [
          { offset: 0, color: '#141416' },
          { offset: 0.5, color: '#27272a' },
          { offset: 1, color: '#101012' }
        ]
      }
    },
    preview: 'linear-gradient(180deg, #141416, #27272a, #101012)'
  },
  {
    name: 'Emerald Night',
    background: {
      kind: 'Gradient',
      data: {
        kind: 'linear',
        angle_deg: 135,
        stops: [
          { offset: 0, color: '#022c22' },
          { offset: 0.5, color: '#064e3b' },
          { offset: 1, color: '#021a14' }
        ]
      }
    },
    preview: 'linear-gradient(135deg, #022c22, #064e3b, #021a14)'
  },
  {
    name: 'Sunset Flare',
    background: {
      kind: 'Gradient',
      data: {
        kind: 'linear',
        angle_deg: 180,
        stops: [
          { offset: 0, color: '#3b0764' },
          { offset: 0.5, color: '#701a75' },
          { offset: 1, color: '#831843' }
        ]
      }
    },
    preview: 'linear-gradient(180deg, #3b0764, #701a75, #831843)'
  },
  {
    name: 'Sunset Horizon',
    background: {
      kind: 'Gradient',
      data: {
        kind: 'linear',
        angle_deg: 160,
        stops: [
          { offset: 0, color: '#2d1b4e' },
          { offset: 0.5, color: '#c74b50' },
          { offset: 1, color: '#f5a25d' }
        ]
      }
    },
    preview: 'linear-gradient(160deg, #2d1b4e, #c74b50, #f5a25d)'
  },
  {
    name: 'Rose Gold Dawn',
    background: {
      kind: 'Gradient',
      data: {
        kind: 'linear',
        angle_deg: 135,
        stops: [
          { offset: 0, color: '#3a1c2b' },
          { offset: 0.5, color: '#a24b6f' },
          { offset: 1, color: '#e8a87c' }
        ]
      }
    },
    preview: 'linear-gradient(135deg, #3a1c2b, #a24b6f, #e8a87c)'
  },
  {
    name: 'Emerald Deep',
    background: {
      kind: 'Gradient',
      data: {
        kind: 'linear',
        angle_deg: 150,
        stops: [
          { offset: 0, color: '#032a1e' },
          { offset: 0.5, color: '#0f5c46' },
          { offset: 1, color: '#1a8a6e' }
        ]
      }
    },
    preview: 'linear-gradient(150deg, #032a1e, #0f5c46, #1a8a6e)'
  },
  {
    name: 'Amber Glow',
    background: {
      kind: 'Gradient',
      data: {
        kind: 'linear',
        angle_deg: 160,
        stops: [
          { offset: 0, color: '#1a1305' },
          { offset: 0.5, color: '#6b4a1a' },
          { offset: 1, color: '#d4941e' }
        ]
      }
    },
    preview: 'linear-gradient(160deg, #1a1305, #6b4a1a, #d4941e)'
  },
  {
    name: 'Twilight Violet',
    background: {
      kind: 'Gradient',
      data: {
        kind: 'linear',
        angle_deg: 160,
        stops: [
          { offset: 0, color: '#0d0221' },
          { offset: 0.5, color: '#3d1e6d' },
          { offset: 1, color: '#6b2fa3' }
        ]
      }
    },
    preview: 'linear-gradient(160deg, #0d0221, #3d1e6d, #6b2fa3)'
  }
];

// Animated backgrounds: layered drifting radial-gradient "particle" fields
// (dust, snow, stars, clouds, aurora bands) over a base gradient. Represented
// as a Solid background whose `data` is a "pattern:<name>" marker instead of
// a literal CSS color — the marker round-trips through the flat legacy
// `background` string field exactly like a real color would (see
// applyBackgroundToSlide), and is resolved into the actual animated
// background-image + CSS animation by applyResolvedBackground
// (core/presentation_helpers.ts) wherever a slide is actually rendered. The
// `preview` field mirrors the pattern's own base gradient for any context
// that only wants a static swatch; the picker itself renders these with the
// real animation running (see toolbar.ts's Animated tab).
export const ANIMATED_PATTERN_PRESETS: BackgroundPreset[] = [
  {
    name: 'Floating Dust',
    background: { kind: 'Solid', data: 'pattern:floating-dust' },
    preview: 'linear-gradient(160deg, #2b2013, #4a3820, #6b5227)'
  },
  {
    name: 'Drifting Clouds',
    background: { kind: 'Solid', data: 'pattern:drifting-clouds' },
    preview: 'linear-gradient(180deg, #3a5a8a, #6683b0, #9db4d6)'
  },
  {
    name: 'Gentle Snowfall',
    background: { kind: 'Solid', data: 'pattern:gentle-snowfall' },
    preview: 'linear-gradient(180deg, #0d1b2a, #1b3a5c, #2c5480)'
  },
  {
    name: 'Aurora Waves',
    background: { kind: 'Solid', data: 'pattern:aurora-waves' },
    preview: 'linear-gradient(180deg, #050814, #0a1128, #030509)'
  },
  {
    name: 'Starfield Drift',
    background: { kind: 'Solid', data: 'pattern:starfield-drift' },
    preview: 'radial-gradient(ellipse at center, #0a0e27, #000000)'
  }
];

export interface FontFamilyGroup {
  label: string;
  fonts: string[];
}

// Curated web-safe font stack, grouped by category for the font picker UI.
// Deliberately not OS font enumeration (the Font Access API is Chromium-only,
// permission-gated, and unreliable inside this app's webview).
export const FONT_FAMILY_GROUPS: FontFamilyGroup[] = [
  {
    label: 'Sans-Serif',
    fonts: ['Arial', 'Helvetica', 'Verdana', 'Tahoma', 'Trebuchet MS', 'Segoe UI', 'Roboto', 'Open Sans', 'Montserrat', 'Lucida Sans', 'Century Gothic']
  },
  {
    label: 'Serif',
    fonts: ['Georgia', 'Times New Roman', 'Garamond', 'Palatino', 'Book Antiqua', 'Merriweather', 'Playfair Display']
  },
  {
    label: 'Display',
    fonts: ['Impact', 'Oswald', 'Bebas Neue']
  },
  {
    label: 'Monospace',
    fonts: ['Courier New']
  }
];
