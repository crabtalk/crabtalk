//! The guest's allocator, initialized the first time something allocates.
//!
//! The host knows where the heap is; the guest has to learn it. The obvious
//! way — the host enters the guest to hand the bounds over — costs a second
//! entry, and entering a guest is around 13µs against ~30ns for a host call.
//! So the guest asks instead, from inside the entry it is already in, the first
//! time it allocates. A harness that never allocates never asks, which is why
//! there is nothing to declare.
#![cfg(all(feature = "alloc", target_arch = "riscv64"))]

use crate::{
    abi::{HOST_HEAP_SIZE, HOST_HEAP_START},
    sys,
};
use core::alloc::{GlobalAlloc, Layout};
use linked_list_allocator::LockedHeap;

#[global_allocator]
static ALLOCATOR: Heap = Heap(LockedHeap::empty());

struct Heap(LockedHeap);

unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut heap = self.0.lock();
        if heap.size() == 0 {
            let start = sys::call0(HOST_HEAP_START) as usize;
            let size = sys::call0(HOST_HEAP_SIZE) as usize;
            // Safety: the bounds come from the host, which committed exactly
            // this region for this guest and nothing else can reach it.
            unsafe { heap.init(start as *mut u8, size) };
        }
        heap.allocate_first_fit(layout)
            .map_or(core::ptr::null_mut(), |p| p.as_ptr())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(ptr) = core::ptr::NonNull::new(ptr) {
            unsafe { self.0.lock().deallocate(ptr, layout) };
        }
    }
}
