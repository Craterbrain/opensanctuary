use std::collections::VecDeque;
use std::sync::RwLock;
use tokio::sync::broadcast;
use crate::core::events::{EventEnvelope, ShowEvent};

pub struct EventLog {
    history: RwLock<VecDeque<EventEnvelope>>,
    max_history: usize,
    sequence_counter: RwLock<u64>,
    broadcaster: broadcast::Sender<EventEnvelope>,
}

impl EventLog {
    pub fn new(capacity: usize) -> Self {
        Self::with_initial_sequence(capacity, 0)
    }

    pub fn with_initial_sequence(capacity: usize, initial_sequence: u64) -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self {
            history: RwLock::new(VecDeque::with_capacity(capacity)),
            max_history: capacity,
            sequence_counter: RwLock::new(initial_sequence),
            broadcaster: tx,
        }
    }

    pub fn load_history(&self, events: &[EventEnvelope]) {
        let mut hist = self.history.write().unwrap_or_else(|e| e.into_inner());
        hist.clear();
        let start = if events.len() > self.max_history {
            events.len() - self.max_history
        } else {
            0
        };
        for env in &events[start..] {
            hist.push_back(env.clone());
        }
        let max_seq = events.iter().map(|e| e.sequence_number).max().unwrap_or(0);
        let mut seq_guard = self.sequence_counter.write().unwrap_or_else(|e| e.into_inner());
        if max_seq > *seq_guard {
            *seq_guard = max_seq;
        }
    }

    pub fn set_sequence(&self, seq: u64) {
        let mut seq_guard = self.sequence_counter.write().unwrap_or_else(|e| e.into_inner());
        if seq > *seq_guard {
            *seq_guard = seq;
        }
    }

    pub fn append(&self, event: ShowEvent) -> EventEnvelope {
        let mut seq_guard = self.sequence_counter.write().unwrap_or_else(|e| e.into_inner());
        *seq_guard += 1;
        let sequence_number = *seq_guard;

        let envelope = EventEnvelope {
            sequence_number,
            event_id: uuid::Uuid::new_v4().to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
            event,
        };

        {
            let mut hist = self.history.write().unwrap_or_else(|e| e.into_inner());
            if hist.len() >= self.max_history {
                hist.pop_front();
            }
            hist.push_back(envelope.clone());
        }

        // Broadcast to all active subscribers only if receivers exist
        if self.broadcaster.receiver_count() > 0 {
            let _ = self.broadcaster.send(envelope.clone());
        }

        envelope
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.broadcaster.subscribe()
    }

    pub fn current_sequence(&self) -> u64 {
        *self.sequence_counter.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn get_events_since(&self, since_sequence: u64) -> Vec<EventEnvelope> {
        let hist = self.history.read().unwrap_or_else(|e| e.into_inner());
        hist.iter()
            .filter(|e| e.sequence_number > since_sequence)
            .cloned()
            .collect()
    }

    pub fn all_events(&self) -> Vec<EventEnvelope> {
        let hist = self.history.read().unwrap_or_else(|e| e.into_inner());
        hist.iter().cloned().collect()
    }
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new(2000)
    }
}
