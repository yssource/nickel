use std::{cell::UnsafeCell, mem, ptr};

use super::{
    blocks::{Blocks, Header, BLOCK_SIZE},
    RootAt, TraceAt,
};

pub mod gc_stats {
    use std::sync::atomic::AtomicUsize;

    pub static BLOCK_COUNT: AtomicUsize = AtomicUsize::new(0);
}

thread_local! {
    pub static NURSERY: &'static UnsafeCell<Blocks> = Box::leak(Box::new(UnsafeCell::new(Blocks::default())));
    pub static ROOTS: UnsafeCell<std::collections::HashSet<std::rc::Rc<RootAt>>> = UnsafeCell::new(Default::default());
    /// This GC is only a two space for now.
    pub static MARKER: UnsafeCell<bool> = UnsafeCell::new(true);
}

pub unsafe fn run_evac() {
    #[allow(clippy::mutable_key_type)]
    let mut roots = Default::default();
    ROOTS.with(|r| mem::swap(&mut *r.get(), &mut roots));
    let mut new_nursery = Default::default();
    let mut nursery = Blocks::default();
    NURSERY.with(|r| mem::swap(&mut *r.get(), &mut nursery));

    let marker = MARKER.with(|m| {
        let m = &mut *m.get();
        let old_m = *m;
        *m = !old_m;
        old_m
    });

    let mut to_trace = Vec::with_capacity(100);

    roots.iter().for_each(|root_at| {
        let root_old_ptr = root_at.ptr.load(std::sync::atomic::Ordering::Relaxed) as *const u8;
        let root_old_ptr_clone = root_old_ptr;
        let root_trace_at = TraceAt {
            ptr_to_gc: &root_old_ptr,
            trace_fn: root_at.trace_fn,
        };
        let new_ptr = evac(root_trace_at, &mut new_nursery, &mut to_trace, marker);
        assert_eq!(root_old_ptr, root_old_ptr_clone);

        // ~~This could be moved into evac~~, but that would add a unconditional Relaxed store.
        // For now it's better here.
        root_at
            .ptr
            .store(new_ptr as usize, std::sync::atomic::Ordering::Relaxed);

        while let Some(trace_at) = to_trace.pop() {
            let new_ptr = evac(trace_at, &mut new_nursery, &mut to_trace, marker);
            *(trace_at.ptr_to_gc as *mut *const u8) = new_ptr;
        }
    });

    ROOTS.with(|r| mem::swap(&mut *r.get(), &mut roots));
    assert!(roots.is_empty());
    NURSERY.with(|r| mem::swap(&mut *r.get(), &mut new_nursery));
    assert!(new_nursery.blocks.is_empty());
}

unsafe fn evac(
    trace_at: TraceAt,
    new_nursery: &mut Blocks,
    to_trace: &mut Vec<TraceAt>,
    marker: bool,
) -> *const u8 {
    dbg!(&trace_at);
    dbg!(&to_trace);

    let old_ptr = *trace_at.ptr_to_gc;
    dbg!(old_ptr);

    let header_ptr = Header::from_ptr(old_ptr as usize) as *mut Header;
    dbg!(header_ptr);
    let header = &mut *(header_ptr);
    Header::checksum(header);
    dbg!(&header);

    assert!((old_ptr as usize - header_ptr as usize) <= BLOCK_SIZE);
    let already_evaced = header.evaced.get(&old_ptr);
    if let Some(new_ptr) = already_evaced {
        eprintln!("already_evaced: new_ptr: {:?}", *new_ptr);
        *new_ptr
    } else if header.marked == marker && already_evaced.is_none() {
        let new_ptr = new_nursery.alloc(header.info);
        dbg!(new_ptr);
        ptr::copy_nonoverlapping(old_ptr, new_ptr, header.info.size as usize);
        (trace_at.trace_fn)(new_ptr, to_trace);
        assert!(header.evaced.insert(old_ptr, new_ptr).is_none());
        dbg!(&to_trace);
        dbg!(&header.evaced);

        new_ptr as *const u8
    } else {
        panic!("Evac was called on an object in an unmarked block!")
    }
}
