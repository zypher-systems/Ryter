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

#[test]
fn insert_and_get() {
    let mut cache: Cache<&str, i32> = Cache::new(2);
    assert_eq!(cache.insert("a", 1), None);
    assert_eq!(cache.get(&"a"), Some(&1));
    assert_eq!(cache.get(&"b"), None);
    assert_eq!(cache.len(), 1);
}

#[test]
fn a_second_insert_replaces_and_returns_the_old_value() {
    let mut cache: Cache<&str, i32> = Cache::new(2);
    cache.insert("a", 1);
    assert_eq!(cache.insert("a", 2), Some(1));
    assert_eq!(cache.get(&"a"), Some(&2));
    assert_eq!(cache.len(), 1);
}

#[test]
fn the_least_recently_used_entry_goes_first() {
    let mut cache: Cache<&str, i32> = Cache::new(2);
    cache.insert("a", 1);
    cache.insert("b", 2);
    cache.get(&"a");
    cache.insert("c", 3);
    assert_eq!(cache.get(&"b"), None);
    assert_eq!(cache.get(&"a"), Some(&1));
    assert_eq!(cache.get(&"c"), Some(&3));
}

#[test]
fn an_entry_expires_after_its_ttl() {
    let (fake, cell) = clock();
    let mut cache: Cache<&str, i32> = Cache::new(4)
        .with_ttl(Duration::from_secs(10))
        .with_clock(fake);
    cache.insert("a", 1);
    advance(&cell, Duration::from_secs(9));
    assert_eq!(cache.get(&"a"), Some(&1));
    advance(&cell, Duration::from_secs(2));
    assert_eq!(cache.get(&"a"), None);
    assert!(cache.is_empty());
    let _ = keys(&cache);
}
