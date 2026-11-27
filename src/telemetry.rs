use chrono::Utc;
use serde::Serialize;
use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

const MAX_LOG_ENTRIES: usize = 400;

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub id: u64,
    pub timestamp: String,
    pub level: String,
    pub target: String,
    pub message: String,
}

pub struct LogStore {
    entries: Mutex<VecDeque<LogEntry>>,
    next_id: AtomicU64,
}

impl LogStore {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(VecDeque::with_capacity(MAX_LOG_ENTRIES)),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn push(&self, level: &str, target: &str, message: String) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let entry = LogEntry {
            id,
            timestamp: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            level: level.to_string(),
            target: target.to_string(),
            message,
        };
        if let Ok(mut guard) = self.entries.lock() {
            if guard.len() >= MAX_LOG_ENTRIES {
                guard.pop_front();
            }
            guard.push_back(entry);
        }
    }

    pub fn recent(&self, limit: usize, level_filter: Option<&str>) -> Vec<LogEntry> {
        let Ok(guard) = self.entries.lock() else {
            return Vec::new();
        };
        let iter = guard.iter().rev().filter(|entry| match level_filter {
            Some(filter) if !filter.is_empty() && !filter.eq_ignore_ascii_case("all") => {
                entry.level.eq_ignore_ascii_case(filter)
            }
            _ => true,
        });
        let mut items: Vec<LogEntry> = iter.take(limit).cloned().collect();
        items.reverse();
        items
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.entries.lock() {
            guard.clear();
        }
    }
}

impl Default for LogStore {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL_LOG_STORE: OnceLock<Arc<LogStore>> = OnceLock::new();

pub fn global_log_store() -> &'static Arc<LogStore> {
    GLOBAL_LOG_STORE.get_or_init(|| Arc::new(LogStore::new()))
}

struct MessageVisitor {
    message: Option<String>,
    fields: Vec<String>,
}

impl MessageVisitor {
    fn new() -> Self {
        Self {
            message: None,
            fields: Vec::new(),
        }
    }

    fn into_string(self) -> String {
        match (self.message, self.fields.is_empty()) {
            (Some(msg), true) => msg,
            (Some(msg), false) => format!("{} ({})", msg, self.fields.join(", ")),
            (None, false) => self.fields.join(", "),
            (None, true) => String::new(),
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{:?}", value));
        } else {
            self.fields.push(format!("{}={:?}", field.name(), value));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push(format!("{}={}", field.name(), value));
        }
    }
}

pub struct MemoryLogLayer;

impl<S: Subscriber> Layer<S> for MemoryLogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut visitor = MessageVisitor::new();
        event.record(&mut visitor);
        let msg = visitor.into_string();
        if !msg.is_empty() {
            global_log_store().push(meta.level().as_str(), meta.target(), msg);
        }
    }
}
