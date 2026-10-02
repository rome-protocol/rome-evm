//! Global heap allocator for the on-chain program.
//!
//! A bump allocator over the Solana heap frame, plus a free list that recycles
//! large freed blocks. A single iterative leg that makes several big-return
//! calls would otherwise bump past the 256 KiB frame, because a plain bump
//! allocator never reuses freed memory; recycling keeps the high-water bounded.
//!
//! The allocation core (`heap_alloc` / `heap_dealloc`) is parameterized by the
//! heap base address so it can be unit-tested on the host over a plain backing
//! buffer; the `#[global_allocator]` wiring (Solana-only) passes
//! `HEAP_START_ADDRESS`.

// Allocation core — compiled on-chain and under `cargo test`. Excluded from
// plain host builds (e.g. the emulator, which uses the system allocator).
#[cfg(any(all(target_os = "solana", not(feature = "no-entrypoint")), test))]
pub(crate) mod imp {
    use core::alloc::Layout;

    pub const WORD: usize = core::mem::size_of::<usize>();
    /// Reserved prefix at the heap base: `[bump_ptr][free_list_head]`.
    pub const PREFIX: usize = 2 * WORD;
    /// Only blocks at least this large are recycled on the free list; smaller
    /// allocations stay pure bump (reclaimed by the next tx's fresh heap). Must
    /// exceed the 2-word free-list node; the buffers that overflow the 256 KiB
    /// frame (call return data, frame memory) are tens of KiB.
    const POOL_MIN: usize = 1024;

    #[inline]
    unsafe fn load(addr: usize) -> usize {
        core::ptr::read_unaligned(addr as *const usize)
    }
    #[inline]
    unsafe fn store(addr: usize, v: usize) {
        core::ptr::write_unaligned(addr as *mut usize, v)
    }

    #[inline]
    unsafe fn bump(base: usize) -> usize {
        load(base)
    }
    #[inline]
    unsafe fn set_bump(base: usize, v: usize) {
        store(base, v)
    }
    #[inline]
    unsafe fn free_head(base: usize) -> usize {
        load(base + WORD)
    }
    #[inline]
    unsafe fn set_free_head(base: usize, v: usize) {
        store(base + WORD, v)
    }

    /// Allocate `layout` from the heap rooted at `base`. Returns null on
    /// arithmetic overflow; on Solana the last-byte probe faults past the frame.
    pub unsafe fn heap_alloc(base: usize, layout: Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();

        // 1) Recycle an exact-size freed block with compatible alignment. Exact
        // fit keeps it simple and correct: a block is reused only for a request
        // of its own size, so its eventual `dealloc` layout matches what it was
        // allocated with. The list holds only large blocks, so it stays short.
        if size >= POOL_MIN {
            let mut prev = 0usize; // 0 => the head lives in the prefix word
            let mut cur = free_head(base);
            while cur != 0 {
                let next = load(cur + WORD);
                if load(cur) == size && cur & (align - 1) == 0 {
                    if prev == 0 {
                        set_free_head(base, next);
                    } else {
                        store(prev + WORD, next);
                    }
                    return cur as *mut u8;
                }
                prev = cur;
                cur = next;
            }
        }

        // 2) Bump.
        let cur = bump(base);
        let start = if cur == 0 { base + PREFIX } else { cur };
        let mask = align - 1;
        let addr = match start.checked_add(mask) {
            None => return core::ptr::null_mut(),
            Some(a) => a & !mask,
        };
        let end = match addr.checked_add(size) {
            None => return core::ptr::null_mut(),
            Some(e) => e,
        };
        // Probe the last byte. On Solana, accessing past the heap frame faults
        // (the intended OOM signal). On host (tests) this reads within the
        // generously-sized backing buffer the caller provides.
        core::ptr::read_volatile((end - 1) as *const u8);
        set_bump(base, end);
        addr as *mut u8
    }

    /// Free a block rooted at `base`. Large blocks are pushed onto the free list
    /// for reuse; small ones are left to the next tx's fresh heap.
    pub unsafe fn heap_dealloc(base: usize, ptr: *mut u8, layout: Layout) {
        if layout.size() >= POOL_MIN {
            let p = ptr as usize;
            store(p, layout.size()); // node.size
            store(p + WORD, free_head(base)); // node.next = previous head
            set_free_head(base, p); // push
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::alloc::Layout;

        // A generously-sized, word-aligned backing buffer standing in for the
        // Solana heap frame. Prefix words are zeroed (bump == 0 => first alloc
        // starts at base + PREFIX).
        fn with_heap(f: impl FnOnce(usize)) {
            let mut buf = vec![0usize; (1 << 20) / WORD]; // 1 MiB
            let base = buf.as_mut_ptr() as usize;
            unsafe {
                set_bump(base, 0);
                set_free_head(base, 0);
            }
            f(base);
        }

        #[test]
        fn recycles_freed_large_block() {
            with_heap(|base| unsafe {
                let big = Layout::from_size_align(80_000, 1).unwrap();
                let small = Layout::from_size_align(64, 1).unwrap();

                let a = heap_alloc(base, big);
                // A small live allocation now sits ABOVE `a`, so `a` is not the
                // bump top — only free-list reuse (not LIFO) can reclaim it.
                let _s = heap_alloc(base, small);
                heap_dealloc(base, a, big);

                let hw = bump(base);
                let c = heap_alloc(base, big);
                assert_eq!(a as usize, c as usize, "freed 80 KiB block must be recycled");
                assert_eq!(bump(base), hw, "high-water must not grow when recycling");
            });
        }

        #[test]
        fn leg_with_repeated_large_returns_stays_bounded() {
            with_heap(|base| unsafe {
                let big = Layout::from_size_align(80_000, 1).unwrap();
                let small = Layout::from_size_align(32, 1).unwrap();

                // Mirror the ERC165Checker bomb leg: each "call" frees the prior
                // 80 KiB return buffer and allocates a fresh one, plus a small
                // persisting allocation. Without recycling the bump would climb
                // ~80 KiB per iteration (8 * 80 KiB > 600 KiB); with recycling
                // the big blocks are reused and growth stays ~one buffer.
                let mut prev = heap_alloc(base, big);
                let baseline = bump(base);
                for _ in 0..8 {
                    let _persist = heap_alloc(base, small);
                    let next = heap_alloc(base, big);
                    heap_dealloc(base, prev, big);
                    prev = next;
                }
                let growth = bump(base) - baseline;
                assert!(
                    growth < 200_000,
                    "heap must stay bounded across the leg (no-recycle would be >600 KiB), grew {growth}"
                );
            });
        }
    }
}

#[cfg(all(target_os = "solana", not(feature = "no-entrypoint")))]
mod global {
    use super::imp::{heap_alloc, heap_dealloc};
    use solana_program::entrypoint::HEAP_START_ADDRESS;

    #[global_allocator]
    static ALLOC: BumpAllocator = BumpAllocator;

    struct BumpAllocator;

    unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
        unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
            heap_alloc(HEAP_START_ADDRESS as usize, layout)
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
            heap_dealloc(HEAP_START_ADDRESS as usize, ptr, layout)
        }
    }
}
