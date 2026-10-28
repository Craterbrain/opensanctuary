// The shapes that cross the API boundary, mirroring the serde structs in src/core/models.rs (and `StateSnapshot` in
// src/core/engine/mod.rs). JSON has no `undefined`: an absent value is `null`, so these use `| null`, unlike the editor's own `EditorSlide`.
// Type the objects you get from `/api/*` and the WebSocket with these instead of `any`.
import type { SlideElement, SlideBackground, SlideTransition } from '../editor/types.ts';

export interface CcliMetadata {
  song_title?: string | null;
  author?: string | null;
  copyright?: string | null;
  ccli_number?: string | null;
  /** The song's "this is public domain" choice (`Song.public_domain_override`). */
  public_domain?: boolean | null;
}

export interface Slide {
  text: string;
  header?: string | null;
  label?: string | null;
  background?: string | null;
  notes?: string | null;
  tag?: string | null;
  elements?: SlideElement[];
  speaker_notes?: string | null;
  ccli_metadata?: CcliMetadata | null;
  background_v2?: SlideBackground | null;
  transition?: SlideTransition | null;
  slide_document_version?: number;
  duration_seconds?: number | null;
  reference_label?: string | null;
}

export interface ArrangementEntry {
  section_id: string;
  source_slide_index: number;
  background_override?: string | null;
}

/** A licensed Bible passage's lookup details (`LicensedRef`); the text itself is never stored. */
export interface LicensedRef {
  source: string;
  bible_id: number;
  usfm: string;
  tag: string;
  copyright?: string;
  link?: string;
  version: string;
  book: string;
  chapter: number;
  resolved?: boolean;
}

export interface ScheduleItem {
  id: string;
  title: string;
  subtitle?: string | null;
  notes?: string | null;
  item_type: string; // "song" | "scripture" | "presentation" | "media" | "header"
  author_or_ref: string;
  slides: Slide[];
  background?: string | null;
  theme_name?: string | null;
  is_expanded?: boolean;
  is_section_header?: boolean;
  arrangement?: ArrangementEntry[];
  default_slide_duration_seconds?: number | null;
  slideshow_loop?: boolean;
  licensed?: LicensedRef | null;
}

export interface Schedule {
  id: string;
  title: string;
  items: ScheduleItem[];
  is_modified?: boolean;
  schedule_version?: number;
}

export interface ShowState {
  live_item: ScheduleItem | null;
  live_slide_index: number;
  staged_item: ScheduleItem | null;
  staged_slide_index: number;
  is_blackout: boolean;
  is_clear_text: boolean;
  is_logo_override: boolean;
  alert_message: string | null;
  global_theme: string | null;
  global_background: string | null;
  selected_item_index?: number | null;
  sequence_number: number;
  last_event_id: string;
  server_time_ms: number;
  web_stream?: WebStream | null;
  transition?: { type: 'Cut' | 'Crossfade' | 'SlideLeft' | 'SlideRight' | 'WipeDown' | 'Dissolve'; duration_ms?: number } | null;
  media_playback?: MediaPlayback | null;
  /** Last visited slide index per section letter (`V`, `C`, `B`...), for the jump-to-section keys. */
  section_jump_cursor?: Record<string, number>;
  /** Not sent by the server today; read by the background resolver. */
  active_theme?: { background_value?: string | null } | null;
}

export interface WebStream {
  url: string;
  mode: string; // "background" | "pip" | "fullscreen"
}

export interface MediaPlayback {
  is_playing: boolean;
  current_time: number;
  timestamp_ms: number;
  start_at_epoch_ms?: number | null;
  is_looping: boolean;
  is_muted: boolean;
  volume: number;
  sync_version: number;
}

export interface StateSnapshot {
  protocol_version: number;
  sequence_number: number;
  timestamp_ms: number;
  state: ShowState;
  schedule: Schedule;
}

export interface Song {
  id: string;
  title: string;
  alternate_title?: string | null;
  author: string;
  copyright?: string | null;
  ccli_number?: string | null;
  slides: Slide[];
  theme_name?: string | null;
  /** `true`: the operator says it is public domain whatever the copyright text says. */
  public_domain_override?: boolean | null;
}

export interface ScriptureVerse {
  verse_number: number;
  text: string;
}

export interface ScriptureItem {
  id: string;
  book: string;
  chapter: number;
  verse_start: number;
  verse_end: number;
  version: string;
  reference: string;
  verses: ScriptureVerse[];
  theme_name?: string | null;
}

export interface MediaItem {
  id: string;
  name: string;
  media_type: string; // "video" | "image" | "audio"
  file_path: string;
  duration_seconds?: number | null;
  thumbnail_path?: string | null;
  loop_playback?: boolean;
}

export interface Presentation {
  id: string;
  title: string;
  author: string;
  slides: Slide[];
  theme_name?: string | null;
}

/** Slides in the library lists come in older shapes too: a presentation's slide may carry `content` and `title` instead of `text` and `label`. */
export type LibrarySlide = Slide & { content?: string; title?: string };

/** The entries of a library list (songs, scriptures, media, presentations, themes) share one row type: the fields each kind has, all optional. */
export interface LibraryItem {
  id?: string;
  title?: string;
  name?: string;
  reference?: string;
  author?: string;
  slides?: LibrarySlide[];
  verses?: ScriptureVerse[];
  file_path?: string;
  url?: string;
  background?: string;
  theme_name?: string | null;
  item_type?: string;
}

/** A command sent to the show engine: `{ Name: payload }` (or just `"Name"` for one without a payload); see `ShowCommand` in src/core/commands.rs. */
export type ShowCommand = string | { [name: string]: unknown };
