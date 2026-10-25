use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use lrulite::{Cache, Clock};

#[derive(Clone)]
struct Fake(Rc<Cell<Instant>>);

impl Clock for Fake {
    fn now(&self) -> Instant {
        self.0.get()
    }
}

fn clock() -> (Fake, Rc<Cell<Instant>>) {
    let cell = Rc::new(Cell::new(Instant::now()));
    (Fake(cell.clone()), cell)
}

fn advance(cell: &Rc<Cell<Instant>>, by: Duration) {
    cell.set(cell.get() + by);
}

fn keys<V>(cache: &Cache<&'static str, V>) -> Vec<&'static str> {
    cache.iter().map(|(k, _)| *k).collect()
}

fn ttl_cache(capacity: usize, secs: u64) -> (Cache<&'static str, i32>, Rc<Cell<Instant>>) {
    let (fake, cell) = clock();
    let cache = Cache::new(capacity)
        .with_ttl(Duration::from_secs(secs))
        .with_clock(fake);
    (cache, cell)
}

#[test]
fn expiry_is_at_the_boundary_and_get_does_not_extend_it() {
    let (mut cache, cell) = ttl_cache(4, 10);
    cache.insert("a", 1);
    advance(&cell, Duration::from_secs(5));
    assert_eq!(cache.get(&"a"), Some(&1));
    advance(&cell, Duration::from_secs(5));
    // Exactly ttl after the insert: gone, whatever the get at 5 s did.
    assert_eq!(cache.peek(&"a"), None);
    assert_eq!(cache.get(&"a"), None);
    assert_eq!(cache.remove(&"a"), None);
}

#[test]
fn a_second_insert_restarts_the_ttl() {
    let (mut cache, cell) = ttl_cache(4, 10);
    cache.insert("a", 1);
    advance(&cell, Duration::from_secs(8));
    assert_eq!(cache.insert("a", 2), Some(1));
    advance(&cell, Duration::from_secs(8));
    assert_eq!(cache.get(&"a"), Some(&2));
    advance(&cell, Duration::from_secs(2));
    assert_eq!(cache.get(&"a"), None);
}

#[test]
fn an_insert_over_an_expired_key_is_a_new_entry() {
    let (mut cache, cell) = ttl_cache(4, 10);
    cache.insert("a", 1);
    advance(&cell, Duration::from_secs(10));
    assert_eq!(cache.insert("a", 2), None);
    assert_eq!(cache.get(&"a"), Some(&2));
}

#[test]
fn len_and_iter_leave_out_the_expired() {
    let (mut cache, cell) = ttl_cache(4, 10);
    cache.insert("a", 1);
    advance(&cell, Duration::from_secs(6));
    cache.insert("b", 2);
    assert_eq!(cache.len(), 2);
    advance(&cell, Duration::from_secs(5));
    assert_eq!(cache.len(), 1);
    assert_eq!(keys(&cache), ["b"]);
    assert!(!cache.is_empty());
    advance(&cell, Duration::from_secs(5));
    assert!(cache.is_empty());
    assert_eq!(keys(&cache), Vec::<&str>::new());
}

#[test]
fn eviction_takes_an_expired_entry_before_the_least_recent() {
    let (mut cache, cell) = ttl_cache(2, 10);
    cache.insert("old", 1);
    advance(&cell, Duration::from_secs(10));
    cache.insert("recent", 2);
    cache.get(&"recent");
    // "old" is expired; without that it would be "recent" that stays and
    // there would be nothing else to evict. The expired one goes.
    cache.insert("new", 3);
    assert_eq!(cache.len(), 2);
    assert_eq!(cache.peek(&"recent"), Some(&2));
    assert_eq!(cache.peek(&"new"), Some(&3));
}

#[test]
fn get_refreshes_recency_and_peek_does_not() {
    let mut cache: Cache<&str, i32> = Cache::new(3);
    cache.insert("a", 1);
    cache.insert("b", 2);
    cache.insert("c", 3);
    assert_eq!(keys(&cache), ["c", "b", "a"]);
    cache.get(&"a");
    assert_eq!(keys(&cache), ["a", "c", "b"]);
    cache.peek(&"b");
    assert_eq!(keys(&cache), ["a", "c", "b"]);
    let _ = cache.iter().count();
    assert_eq!(keys(&cache), ["a", "c", "b"]);
    cache.insert("d", 4);
    assert_eq!(keys(&cache), ["d", "a", "c"]);
    assert_eq!(cache.peek(&"b"), None);
}

#[test]
fn a_replacing_insert_is_a_use() {
    let mut cache: Cache<&str, i32> = Cache::new(2);
    cache.insert("a", 1);
    cache.insert("b", 2);
    cache.insert("a", 10);
    cache.insert("c", 3);
    assert_eq!(cache.peek(&"b"), None);
    assert_eq!(cache.peek(&"a"), Some(&10));
}

#[test]
fn get_or_insert_with_computes_once_until_expiry() {
    let (mut cache, cell) = ttl_cache(2, 10);
    let mut calls = 0;
    let mut make = |v: i32| {
        calls += 1;
        v
    };
    assert_eq!(*cache.get_or_insert_with("a", || make(1)), 1);
    assert_eq!(*cache.get_or_insert_with("a", || make(2)), 1);
    advance(&cell, Duration::from_secs(10));
    assert_eq!(*cache.get_or_insert_with("a", || make(3)), 3);
    assert_eq!(calls, 2);
    // It evicts like an insert.
    cache.insert("b", 20);
    cache.get(&"a");
    cache.get_or_insert_with("c", || 30);
    assert_eq!(cache.peek(&"b"), None);
    assert_eq!(cache.peek(&"a"), Some(&3));
}

#[test]
fn a_cache_of_capacity_zero_keeps_nothing() {
    let mut cache: Cache<&str, i32> = Cache::new(0);
    assert_eq!(cache.insert("a", 1), None);
    assert_eq!(cache.insert("a", 2), None);
    assert_eq!(cache.get(&"a"), None);
    assert_eq!(cache.len(), 0);
    assert!(cache.is_empty());
    assert_eq!(keys(&cache), Vec::<&str>::new());
}

#[test]
fn remove_returns_a_live_value() {
    let (mut cache, cell) = ttl_cache(4, 10);
    cache.insert("a", 1);
    cache.insert("b", 2);
    assert_eq!(cache.remove(&"a"), Some(1));
    assert_eq!(cache.remove(&"a"), None);
    advance(&cell, Duration::from_secs(10));
    assert_eq!(cache.remove(&"b"), None);
    assert!(cache.is_empty());
}

#[test]
fn owned_keys_and_values_work_too() {
    let mut cache: Cache<String, Vec<u8>> = Cache::new(2);
    cache.insert("k".to_string(), vec![1, 2]);
    assert_eq!(cache.get(&"k".to_string()), Some(&vec![1, 2]));
    let v = cache.get_or_insert_with("j".to_string(), || vec![3]);
    assert_eq!(v, &vec![3]);
}
