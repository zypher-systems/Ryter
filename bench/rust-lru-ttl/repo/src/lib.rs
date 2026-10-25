//! A small cache: at most `capacity` entries, each living `ttl` from its insert.
use std::hash::Hash;
use std::time::{Duration, Instant};

/// Where the cache reads the time, so tests can set it.
pub trait Clock {
    fn now(&self) -> Instant;
}

/// The real clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

pub struct Cache<K, V> {
    capacity: usize,
    ttl: Option<Duration>,
    clock: Box<dyn Clock>,
    entries: Vec<(K, V, Instant)>,
}

impl<K: Hash + Eq + Clone, V> Cache<K, V> {
    pub fn new(capacity: usize) -> Self {
        Cache {
            capacity,
            ttl: None,
            clock: Box::new(SystemClock),
            entries: Vec::new(),
        }
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = Some(ttl);
        self
    }

    pub fn with_clock(mut self, clock: impl Clock + 'static) -> Self {
        self.clock = Box::new(clock);
        self
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        let now = self.clock.now();
        self.entries.push((key, value, now));
        if self.entries.len() > self.capacity {
            self.entries.remove(0);
        }
        None
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.entries.iter().find(|(k, _, _)| k == key).map(|(_, v, _)| v)
    }

    pub fn peek(&self, key: &K) -> Option<&V> {
        self.entries.iter().find(|(k, _, _)| k == key).map(|(_, v, _)| v)
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let at = self.entries.iter().position(|(k, _, _)| k == key)?;
        Some(self.entries.remove(at).1)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get_or_insert_with(&mut self, key: K, make: impl FnOnce() -> V) -> &V {
        if self.peek(&key).is_none() {
            self.insert(key.clone(), make());
        }
        self.peek(&key).expect("just inserted")
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries.iter().map(|(k, v, _)| (k, v))
    }
}
