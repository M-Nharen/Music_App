//! Learn mode (Part 3): the theory overlay, lessons and ear-training games.
//!
//! Everything in here is pure (no UI, no audio) so it can be tested. The
//! Learn tab in `ui/app/learn.rs` turns it into screens, lights up keys and
//! plays the examples.

pub mod analysis;
pub mod ear;
pub mod intervals;
pub mod lessons;

/// Tiny xorshift random generator, so the games don't need a crate.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }

    /// Seeded from the clock.
    pub fn from_time() -> Rng {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Rng::new(t ^ 0x2545_F491_4F6C_DD1D)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// 0..n (n > 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            Some(&items[self.below(items.len())])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_spreads() {
        let mut r = Rng::new(42);
        let mut seen = [0; 5];
        for _ in 0..500 {
            seen[r.below(5)] += 1;
        }
        assert!(seen.iter().all(|n| *n > 50), "{seen:?}");
    }
}
