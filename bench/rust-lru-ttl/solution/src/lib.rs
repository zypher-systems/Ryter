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

struct Entry<V> {
    value: V,
    inserted: Instant,
    /// Higher is more recent.
    used: u64,
}

pub struct Cache<K, V> {
    capacity: usize,
    ttl: Option<Duration>,
    clock: Box<dyn Clock>,
    entries: std::collections::HashMap<K, Entry<V>>,
    tick: u64,
}

impl<K: Hash + Eq + Clone, V> Cache<K, V> {
    pub fn new(capacity: usize) -> Self {
        Cache {
            capacity,
            ttl: None,
            clock: Box::new(SystemClock),
            entries: std::collections::HashMap::new(),
            tick: 0,
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

    fn expired(&self, entry: &Entry<V>, now: Instant) -> bool {
        self.ttl.is_some_and(|ttl| now >= entry.inserted + ttl)
    }

    /// Drop every expired entry.
    fn purge(&mut self) {
        let now = self.clock.now();
        let Some(ttl) = self.ttl else {
            return;
        };
        self.entries.retain(|_, e| now < e.inserted + ttl);
    }

    fn next_tick(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    /// Make room for one new entry: an expired one first, else the least
    /// recently used.
    fn make_room(&mut self) {
        let now = self.clock.now();
        if self.entries.len() < self.capacity {
            return;
        }
        if let Some(k) = self
            .entries
            .iter()
            .find(|(_, e)| self.expired(e, now))
            .map(|(k, _)| k.clone())
        {
            self.entries.remove(&k);
            return;
        }
        if let Some(k) = self
            .entries
            .iter()
            .min_by_key(|(_, e)| e.used)
            .map(|(k, _)| k.clone())
        {
            self.entries.remove(&k);
        }
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        if self.capacity == 0 {
            return None;
        }
        let now = self.clock.now();
        let tick = self.next_tick();
        if let Some(e) = self.entries.get_mut(&key) {
            if !self.ttl.is_some_and(|ttl| now >= e.inserted + ttl) {
                let old = std::mem::replace(&mut e.value, value);
                e.inserted = now;
                e.used = tick;
                return Some(old);
            }
            self.entries.remove(&key);
        }
        self.make_room();
        self.entries.insert(
            key,
            Entry {
                value,
                inserted: now,
                used: tick,
            },
        );
        None
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        let now = self.clock.now();
        let tick = self.tick + 1;
        let ttl = self.ttl;
        let live = match self.entries.get_mut(key) {
            Some(e) if ttl.is_some_and(|ttl| now >= e.inserted + ttl) => false,
            Some(e) => {
                e.used = tick;
                true
            }
            None => return None,
        };
        if !live {
            self.entries.remove(key);
            return None;
        }
        self.tick = tick;
        self.entries.get(key).map(|e| &e.value)
    }

    pub fn peek(&self, key: &K) -> Option<&V> {
        let now = self.clock.now();
        self.entries
            .get(key)
            .filter(|e| !self.expired(e, now))
            .map(|e| &e.value)
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let now = self.clock.now();
        let e = self.entries.remove(key)?;
        (!self.expired(&e, now)).then_some(e.value)
    }

    pub fn len(&self) -> usize {
        let now = self.clock.now();
        self.entries.values().filter(|e| !self.expired(e, now)).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get_or_insert_with(&mut self, key: K, make: impl FnOnce() -> V) -> &V {
        if self.get(&key).is_none() {
            self.insert(key.clone(), make());
        }
        if self.capacity == 0 {
            // Nothing is kept: give the value a place to live for this call.
            self.entries.clear();
            self.purge();
        }
        self.entries
            .get(&key)
            .map(|e| &e.value)
            .expect("a cache of capacity zero cannot lend a value")
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        let now = self.clock.now();
        let mut live: Vec<&Entry<V>> = Vec::new();
        let mut rows: Vec<(&K, &Entry<V>)> = self
            .entries
            .iter()
            .filter(|(_, e)| !self.expired(e, now))
            .collect();
        rows.sort_by(|a, b| b.1.used.cmp(&a.1.used));
        live.clear();
        rows.into_iter().map(|(k, e)| (k, &e.value))
    }
}
