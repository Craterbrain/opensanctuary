use serde::{Deserialize, Serialize};
use crate::core::models::Song;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeniusSongHit {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub url: String,
    pub thumbnail: Option<String>,
    pub ccli_number: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeniusMultiSearchResponse {
    response: Option<GeniusResponsePayload>,
}

#[derive(Debug, Deserialize)]
struct GeniusResponsePayload {
    sections: Option<Vec<GeniusSection>>,
}

#[derive(Debug, Deserialize)]
struct GeniusSection {
    #[serde(rename = "type")]
    section_type: Option<String>,
    hits: Option<Vec<GeniusHit>>,
}

#[derive(Debug, Deserialize)]
struct GeniusHit {
    result: Option<GeniusResultItem>,
}

#[derive(Debug, Deserialize)]
struct GeniusResultItem {
    id: Option<u64>,
    title: Option<String>,
    artist_names: Option<String>,
    url: Option<String>,
    song_art_image_thumbnail_url: Option<String>,
    header_image_thumbnail_url: Option<String>,
}

pub static CHRISTIAN_ARTISTS_KEYWORDS: &[&str] = &[
    "worship", "christian", "gospel", "praise", "hymn", "church", "jesus", "god", "lord", "holy",
    "faith", "grace", "savior", "christ", "cross", "hallelu",
];

pub struct GeniusImporter;

impl GeniusImporter {
    /// Search Genius for Christian & Worship song lyrics
    pub async fn search_christian_lyrics(
        query: &str,
    ) -> Result<Vec<GeniusSongHit>, Box<dyn std::error::Error + Send + Sync>> {
        let q_clean = query.trim();
        if q_clean.is_empty() {
            return Ok(Vec::new());
        }

        let q_lower = q_clean.to_lowercase();
        let already_has_christian_kw = CHRISTIAN_ARTISTS_KEYWORDS.iter().any(|k| q_lower.contains(k));
        
        let search_terms = if already_has_christian_kw {
            q_clean.to_string()
        } else {
            format!("{} christian worship", q_clean)
        };

        let encoded = urlencoding::encode(&search_terms);
        let url = format!("https://genius.com/api/search/multi?q={}", encoded);

        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()?;

        let resp = client.get(&url).send().await?.error_for_status()?;
        let data: GeniusMultiSearchResponse = resp.json().await?;

        let mut hits = Vec::new();
        let mut seen_urls = std::collections::HashSet::new();

        if let Some(payload) = data.response {
            if let Some(sections) = payload.sections {
                for section in sections {
                    let st = section.section_type.as_deref().unwrap_or("");
                    if st == "song" || st == "top_hit" {
                        if let Some(section_hits) = section.hits {
                            for hit in section_hits {
                                if let Some(res) = hit.result {
                                    let song_url = res.url.unwrap_or_default();
                                    if !song_url.contains("lyrics") && !song_url.contains("annotated") {
                                        continue;
                                    }
                                    if seen_urls.contains(&song_url) {
                                        continue;
                                    }

                                    let title = res.title.unwrap_or_else(|| "Unknown Title".into());
                                    let artist = res.artist_names.unwrap_or_else(|| "Christian Artist".into());
                                    let combined = format!("{} {}", title, artist).to_lowercase();

                                    let is_relevant = already_has_christian_kw 
                                        || CHRISTIAN_ARTISTS_KEYWORDS.iter().any(|k| combined.contains(k))
                                        || song_url.to_lowercase().contains("worship")
                                        || song_url.to_lowercase().contains("christian");

                                    if title.contains("Release Calendar") || title.contains("Tracklist") {
                                        continue;
                                    }

                                    if is_relevant || hits.len() < 5 {
                                        seen_urls.insert(song_url.clone());
                                        let hit_id = res.id.map(|i| format!("genius_{}", i)).unwrap_or_else(|| {
                                            format!("genius_{}", uuid::Uuid::new_v4().simple())
                                        });

                                        hits.push(GeniusSongHit {
                                            id: hit_id,
                                            title,
                                            artist,
                                            url: song_url,
                                            thumbnail: res.song_art_image_thumbnail_url.or(res.header_image_thumbnail_url),
                                            ccli_number: None,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(hits)
    }

    /// Fetch HTML from Genius song page, extract lyrics, CCLI number, and format into structured Song model
    pub async fn fetch_and_import_lyrics(
        url: &str,
        custom_title: Option<&str>,
        custom_artist: Option<&str>,
    ) -> Result<Song, Box<dyn std::error::Error + Send + Sync>> {
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()?;

        let html = client.get(url).send().await?.error_for_status()?.text().await?;

        let mut extracted_chunks = Vec::new();
        let marker = "data-lyrics-container=\"true\"";
        let mut rest = html.as_str();

        while let Some(pos) = rest.find(marker) {
            let after_marker = &rest[pos + marker.len()..];
            if let Some(open_gt) = after_marker.find('>') {
                let inside_div = &after_marker[open_gt + 1..];
                if let Some(close_div) = inside_div.find("</div>") {
                    extracted_chunks.push(&inside_div[..close_div]);
                    rest = &inside_div[close_div + 6..];
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        let mut raw_combined = extracted_chunks.join("\n");
        if raw_combined.is_empty() {
            if let Some(legacy_start) = html.find("class=\"lyrics\"") {
                let after = &html[legacy_start..];
                if let Some(open_gt) = after.find('>') {
                    let inside = &after[open_gt + 1..];
                    if let Some(close_div) = inside.find("</div>") {
                        raw_combined = inside[..close_div].to_string();
                    }
                }
            }
        }

        let clean_lyrics = strip_html_tags_and_decode(&raw_combined);

        let title = custom_title.unwrap_or("Worship Song");
        let artist = custom_artist.unwrap_or("Christian Artist");

        let mut song = parse_genius_lyrics_to_song(title, artist, &clean_lyrics);

        // Dynamically extract CCLI number from the page text / annotations if present
        if let Some(ccli) = extract_ccli_from_text(&html) {
            song.ccli_number = Some(ccli);
        }

        Ok(song)
    }
}

/// Extract CCLI number digits from free-form text or metadata dynamically
pub fn extract_ccli_from_text(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let mut search_from = 0;

    while let Some(idx) = lower[search_from..].find("ccli") {
        let actual_idx = search_from + idx;
        let snippet = &lower[actual_idx..];
        
        let rest = &snippet[4..];
        let mut num_str = String::new();
        let mut skipped = 0;

        for (i, c) in rest.char_indices() {
            if c.is_ascii_digit() {
                for next_c in rest[i..].chars() {
                    if next_c.is_ascii_digit() {
                        num_str.push(next_c);
                    } else {
                        break;
                    }
                }
                break;
            }
            skipped += 1;
            if skipped > 25 {
                break;
            }
        }

        if num_str.len() >= 4 && num_str.len() <= 10 {
            return Some(num_str);
        }

        search_from = actual_idx + 4;
    }

    None
}

fn strip_html_tags_and_decode(html: &str) -> String {
    let replaced_br = html
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</p>", "\n\n")
        .replace("</div>", "\n");

    let mut in_tag = false;
    let mut result = String::with_capacity(replaced_br.len());
    for ch in replaced_br.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(ch);
        }
    }

    result
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}

/// Parse raw Genius lyrics containing [Verse 1], [Chorus], [Bridge], etc. into structured SongSlide items
pub fn parse_genius_lyrics_to_song(title: &str, artist: &str, raw_lyrics: &str) -> Song {
    let mut song = Song::new(title, artist);
    song.copyright = Some("Genius Christian Lyrics".into());

    let lines: Vec<&str> = raw_lyrics.lines().collect();
    let mut current_label = "Verse 1".to_string();
    let mut current_tag = "V1".to_string();
    let mut current_text_lines: Vec<String> = Vec::new();
    let mut verse_counter = 1;
    let mut chorus_counter = 1;
    let mut bridge_counter = 1;

    let flush_section = |song: &mut Song, label: &str, tag: &str, lines: &mut Vec<String>| {
        let text = lines.join("\n").trim().to_string();
        if !text.is_empty() {
            song.add_slide(label, tag, text);
        }
        lines.clear();
    };

    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            flush_section(&mut song, &current_label, &current_tag, &mut current_text_lines);

            let header = &trimmed[1..trimmed.len() - 1];
            let header_lower = header.to_lowercase();
            if header_lower.contains("chorus") {
                current_label = format!("Chorus {}", chorus_counter);
                current_tag = format!("C{}", chorus_counter);
                chorus_counter += 1;
            } else if header_lower.contains("bridge") {
                current_label = format!("Bridge {}", bridge_counter);
                current_tag = format!("B{}", bridge_counter);
                bridge_counter += 1;
            } else if header_lower.contains("outro") || header_lower.contains("ending") {
                current_label = "Ending".to_string();
                current_tag = "E1".to_string();
            } else if header_lower.contains("verse") {
                current_label = format!("Verse {}", verse_counter);
                current_tag = format!("V{}", verse_counter);
                verse_counter += 1;
            } else {
                current_label = header.to_string();
                current_tag = format!("V{}", verse_counter);
                verse_counter += 1;
            }
        } else if (!trimmed.is_empty() || !current_text_lines.is_empty())
            && !trimmed.contains("Contributors")
            && !trimmed.contains("Translations")
            && !trimmed.contains("Embed")
        {
            current_text_lines.push(line.to_string());
        }
    }

    flush_section(&mut song, &current_label, &current_tag, &mut current_text_lines);

    if song.slides.is_empty() {
        let chunks = raw_lyrics.split("\n\n");
        for (idx, chunk) in chunks.enumerate() {
            let t = chunk.trim();
            if !t.is_empty() && !t.contains("Contributors") && !t.contains("Embed") {
                song.add_slide(format!("Verse {}", idx + 1), format!("V{}", idx + 1), t);
            }
        }
    }

    song
}
