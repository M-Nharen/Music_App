//! Tests only: counts heap allocations and frees per thread, so tests can
//! check that audio-thread code never touches the allocator.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static COUNT: Cell<u64> = const { Cell::new(0) };
}

fn bump() {
    let _ = COUNT.try_with(|c| c.set(c.get() + 1));
}

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        bump();
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// Allocations + frees made by this thread so far.
pub fn count() -> u64 {
    COUNT.with(|c| c.get())
}

#[cfg(test)]
mod tests {
    #[test]
    fn counts_this_threads_allocations() {
        let before = super::count();
        let v = std::hint::black_box(vec![1u8; 10]);
        drop(v);
        assert!(super::count() >= before + 2);
    }
}
