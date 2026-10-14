//! The notification bus: one place every source (the host itself, an approved plugin) posts to
//! and every console reads from. Kept in memory (a short history, not an archive); consoles pick
//! new entries up by polling `GET /api/notifications` (`wait=` holds the request open).
//!
//! A source is a string: `host`, or `plugin:<id>` (set by the host from the caller's token, never
//! by the plugin). Text is plain text only -- consoles render it with `textContent` -- and every
//! field is length-capped, so a plugin can inform the operator but can't dress up as the host or
//! flood the screen.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

const HISTORY: usize = 200;
const TITLE_MAX: usize = 80;
const BODY_MAX: usize = 400;
const KEY_MAX: usize = 64;
/// A source may post at most this many notifications per `RATE_WINDOW`.
const RATE_LIMIT: usize = 10;
const RATE_WINDOW: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Info,
    Success,
    Warning,
    Error,
}

impl Level {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "info" => Some(Self::Info),
            "success" => Some(Self::Success),
            "warning" => Some(Self::Warning),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Notification {
    pub id: u64,
    pub ts_ms: i64,
    /// `host` or `plugin:<id>`.
    pub source: String,
    /// What the console shows next to it ("Video Downloader").
    pub source_name: String,
    pub level: Level,
    pub title: String,
    pub body: String,
    /// Stays on screen until dismissed (errors and warnings only; see `post`).
    pub sticky: bool,
    /// Consoles use it to replace an earlier entry the host just dropped for the same key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// What a source supplies. The host fills in the rest.
#[derive(Debug, Default)]
pub struct NewNotification {
    pub level: Option<Level>,
    pub title: String,
    pub body: String,
    /// A later post with the same source and key replaces the earlier one (progress, "still
    /// failing") instead of stacking up.
    pub key: Option<String>,
    pub sticky: bool,
}

struct Inner {
    items: VecDeque<Notification>,
    next_id: u64,
    recent: std::collections::HashMap<String, VecDeque<Instant>>,
}

pub struct NotificationBus {
    inner: Mutex<Inner>,
    latest: tokio::sync::watch::Sender<u64>,
}

impl Default for NotificationBus {
    fn default() -> Self {
        Self::new()
    }
}

fn clip(s: &str, max: usize) -> String {
    // Control characters (other than newlines in the body) have no business in a notification.
    s.chars().filter(|c| !c.is_control() || *c == '\n').take(max).collect::<String>().trim().to_string()
}

impl NotificationBus {
    pub fn new() -> Self {
        let (latest, _) = tokio::sync::watch::channel(0);
        Self { inner: Mutex::new(Inner { items: VecDeque::new(), next_id: 1, recent: Default::default() }), latest }
    }

    /// Posts a notification. Errors when the source is over its rate limit or sent nothing.
    pub fn post(&self, source: &str, source_name: &str, n: NewNotification) -> Result<u64, String> {
        let title = clip(&n.title, TITLE_MAX).replace('\n', " ");
        let body = clip(&n.body, BODY_MAX);
        if title.is_empty() && body.is_empty() {
            return Err("a notification needs a title or a message".to_string());
        }
        let level = n.level.unwrap_or(Level::Info);
        let key = n.key.as_deref().map(|k| clip(k, KEY_MAX)).filter(|k| !k.is_empty());
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let window = g.recent.entry(source.to_string()).or_default();
        while window.front().is_some_and(|t| now.duration_since(*t) > RATE_WINDOW) {
            window.pop_front();
        }
        if window.len() >= RATE_LIMIT {
            return Err("posting notifications too fast".to_string());
        }
        window.push_back(now);
        if g.recent.len() > 256 {
            g.recent.retain(|_, w| w.back().is_some_and(|t| now.duration_since(*t) <= RATE_WINDOW));
        }
        if let Some(k) = &key {
            g.items.retain(|item| !(item.source == source && item.key.as_deref() == Some(k.as_str())));
        }
        let id = g.next_id;
        g.next_id += 1;
        let item = Notification {
            id,
            ts_ms: chrono::Utc::now().timestamp_millis(),
            source: source.to_string(),
            source_name: clip(source_name, 60),
            level,
            title,
            body,
            sticky: n.sticky && matches!(level, Level::Warning | Level::Error),
            key,
        };
        g.items.push_back(item);
        while g.items.len() > HISTORY {
            g.items.pop_front();
        }
        drop(g);
        self.latest.send_replace(id);
        Ok(id)
    }

    /// Everything newer than `after`, oldest first.
    pub fn since(&self, after: u64) -> Vec<Notification> {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.items.iter().filter(|n| n.id > after).cloned().collect()
    }

    pub fn latest_id(&self) -> u64 {
        *self.latest.borrow()
    }

    /// Like `since`, but waits up to `wait` for something newer to arrive.
    pub async fn since_wait(&self, after: u64, wait: Duration) -> Vec<Notification> {
        let mut rx = self.latest.subscribe();
        let found = self.since(after);
        if !found.is_empty() || wait.is_zero() {
            return found;
        }
        let _ = tokio::time::timeout(wait, rx.wait_for(|latest| *latest > after)).await;
        self.since(after)
    }

    pub fn dismiss(&self, id: u64) -> bool {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let before = g.items.len();
        g.items.retain(|n| n.id != id);
        g.items.len() != before
    }

    /// Clears everything from a source, or everything when `source` is `None`.
    pub fn clear(&self, source: Option<&str>) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match source {
            Some(s) => g.items.retain(|n| n.source != s),
            None => g.items.clear(),
        }
    }

    /// Convenience for the host's own notices.
    pub fn post_host(&self, level: Level, title: &str, body: &str) {
        let _ = self.post("host", "OpenSanctuary", NewNotification { level: Some(level), title: title.into(), body: body.into(), ..Default::default() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(title: &str) -> NewNotification {
        NewNotification { title: title.into(), ..Default::default() }
    }

    #[test]
    fn posts_in_order_and_filters_by_id() {
        let bus = NotificationBus::new();
        let a = bus.post("host", "OS", n("a")).unwrap();
        let b = bus.post("host", "OS", n("b")).unwrap();
        assert!(b > a);
        assert_eq!(bus.since(0).len(), 2);
        assert_eq!(bus.since(a).len(), 1);
        assert_eq!(bus.since(b).len(), 0);
    }

    #[test]
    fn key_replaces_and_caps_apply() {
        let bus = NotificationBus::new();
        let k = |t: &str| NewNotification { title: t.into(), key: Some("dl".into()), ..Default::default() };
        bus.post("plugin:x", "X", k("1")).unwrap();
        bus.post("plugin:x", "X", k("2")).unwrap();
        bus.post("plugin:y", "Y", k("other source")).unwrap();
        let all = bus.since(0);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].title, "2");
        assert_eq!(all[1].title, "other source");
        let long = bus.post("host", "OS", NewNotification { title: "t".repeat(500), body: "b".repeat(5000), ..Default::default() }).unwrap();
        let item = bus.since(long - 1).pop().unwrap();
        assert_eq!(item.title.len(), TITLE_MAX);
        assert_eq!(item.body.len(), BODY_MAX);
        assert!(bus.post("host", "OS", n("  ")).is_err());
    }

    #[test]
    fn rate_limited_per_source_and_sticky_only_for_problems() {
        let bus = NotificationBus::new();
        for _ in 0..RATE_LIMIT {
            bus.post("plugin:x", "X", n("spam")).unwrap();
        }
        assert!(bus.post("plugin:x", "X", n("spam")).is_err());
        assert!(bus.post("plugin:y", "Y", n("fine")).is_ok());
        let sticky = |level| NewNotification { title: "t".into(), level: Some(level), sticky: true, ..Default::default() };
        let info = bus.post("host", "OS", sticky(Level::Info)).unwrap();
        let err = bus.post("host", "OS", sticky(Level::Error)).unwrap();
        assert!(!bus.since(info - 1)[0].sticky);
        assert!(bus.since(err - 1)[0].sticky);
    }

    #[tokio::test]
    async fn long_poll_wakes_on_post() {
        let bus = std::sync::Arc::new(NotificationBus::new());
        let b2 = bus.clone();
        let waiter = tokio::spawn(async move { b2.since_wait(0, Duration::from_secs(5)).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        bus.post("host", "OS", n("hi")).unwrap();
        assert_eq!(waiter.await.unwrap().len(), 1);
        assert!(bus.since_wait(99, Duration::from_millis(30)).await.is_empty());
    }
}
