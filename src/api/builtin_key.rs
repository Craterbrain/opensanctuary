//! The YouVersion App Key release builds carry (`YVP_APP_KEY` at build time, masked by `build.rs`).
//!
//! The key is added by the *server*, to requests from the YouVersion plugin to `api.youversion.com`, when
//! the plugin hasn't supplied a key of its own. The plugin and its page never see it: it is not in the
//! plugin's files, its keyring or any reply. A build made without the variable carries nothing, and the
//! plugin then asks for the church's own key. See docs/LICENSED_SCRIPTURE.md.

use crate::builtin_key_mask::xor_with_pad;

include!(concat!(env!("OUT_DIR"), "/yvp_key.rs"));

/// The only plugin and host the built-in key is ever used for.
const PLUGIN_ID: &str = "youversion";
const HOST: &str = "api.youversion.com";
const HEADER: &str = "x-yvp-app-key";

/// The key this build carries, if any.
pub fn youversion_key() -> Option<String> {
    if YVP_MASKED.is_empty() {
        return None;
    }
    String::from_utf8(xor_with_pad(YVP_MASKED, &YVP_PAD)).ok().filter(|k| !k.trim().is_empty())
}

/// The header to add to a plugin's `net.fetch`, if the built-in key applies: this plugin, this host, and
/// no key header already supplied (the church's own key wins).
pub fn header_for(plugin_id: &str, url: &str, sent: &[(String, String)]) -> Option<(String, String)> {
    header_with(youversion_key(), plugin_id, url, sent)
}

fn header_with(key: Option<String>, plugin_id: &str, url: &str, sent: &[(String, String)]) -> Option<(String, String)> {
    if plugin_id != PLUGIN_ID || sent.iter().any(|(k, _)| k.eq_ignore_ascii_case(HEADER)) {
        return None;
    }
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    if parsed.scheme() != "https" || !parsed.host_str().is_some_and(|h| h.eq_ignore_ascii_case(HOST)) {
        return None;
    }
    key.map(|k| (HEADER.to_string(), k))
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://api.youversion.com/v1/bibles/111/passages/JHN.3.16";

    #[test]
    fn masking_round_trips_and_hides_the_text() {
        let key = b"abc123-SECRET-key-value";
        let pad = [7u8; 32];
        let masked = xor_with_pad(key, &pad);
        assert_ne!(&masked[..], &key[..]);
        assert!(!masked.windows(6).any(|w| w == b"SECRET"));
        assert_eq!(xor_with_pad(&masked, &pad), key);
        assert_ne!(xor_with_pad(key, &[8u8; 32]), masked, "a different pad masks differently");
        assert_eq!(xor_with_pad(b"", &pad), Vec::<u8>::new());
    }

    #[test]
    fn the_key_goes_only_to_the_youversion_plugin_and_host_and_never_over_a_church_key() {
        let k = || Some("K".to_string());
        assert_eq!(header_with(k(), "youversion", URL, &[]), Some(("x-yvp-app-key".into(), "K".into())));
        assert_eq!(header_with(k(), "genius_lyrics", URL, &[]), None, "another plugin, even with the host granted");
        assert_eq!(header_with(k(), "youversion", "https://example.com/", &[]), None, "another host");
        assert_eq!(header_with(k(), "youversion", "https://api.youversion.com.evil.test/", &[]), None);
        assert_eq!(header_with(k(), "youversion", "http://api.youversion.com/", &[]), None, "not over plain http");
        assert_eq!(header_with(k(), "youversion", URL, &[("X-YVP-App-Key".into(), "church".into())]), None, "their own key wins");
        assert_eq!(header_with(None, "youversion", URL, &[]), None, "a build without a key adds nothing");
    }
}
