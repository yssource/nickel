use std::collections::hash_map::Entry;
use std::sync::atomic::Ordering::Relaxed;
use std::{cell::UnsafeCell, mem, ptr};

use crate::blocks::*;
use crate::internals::gc_stats::{BLOCK_COUNT, POST_BLOCK_COUNT};
use crate::root::*;

pub mod gc_stats {
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

    pub fn thread_block_count() -> usize {
        BLOCK_COUNT.with(|bc| bc.load(Relaxed))
    }

    pub fn thread_post_block_count() -> usize {
        POST_BLOCK_COUNT.with(|bc| bc.load(Relaxed))
    }

    thread_local! {
        pub static BLOCK_COUNT: AtomicUsize = AtomicUsize::new(0);
        pub static POST_BLOCK_COUNT: AtomicUsize = AtomicUsize::new(2);
    }
}

thread_local! {
    pub static NURSERY: &'static UnsafeCell<Blocks> = Box::leak(Box::new(UnsafeCell::new(Blocks::default())));
    /// This GC is only a two space for now.
    pub static MARKER: UnsafeCell<bool> = UnsafeCell::new(true);
}

pub(crate) fn marker() -> bool {
    unsafe { MARKER.with(|m| *m.get()) }
}

pub unsafe fn run_evac() {
    let mut new_nursery = Default::default();
    let mut old_nursery = Blocks::default();
    NURSERY.with(|r| mem::swap(&mut *r.get(), &mut old_nursery));
    let headers = old_nursery
        .blocks
        .iter()
        .flat_map(|(_, blocks)| blocks.iter());

    let marker = MARKER.with(|m| {
        let m = &mut *m.get();
        let old_m = *m;
        *m = !old_m;
        old_m
    });

    let mut to_trace = Vec::with_capacity(100);
    for header in headers {
        // Collect all roots from this block
        let evaced = &*header.evaced.get();
        to_trace.extend(evaced.iter().filter_map(
            |(from_space_ptr, obj_status)| match obj_status {
                ObjectStatus::Rooted(r) => {
                    let inner = r.as_ref();
                    // Make sure we're calling `T::trace` not `Root::trace`.
                    assert_eq!(
                        inner.trace_fn as usize,
                        Header::from_ptr((*from_space_ptr) as usize)
                            .as_ref()
                            .unwrap()
                            .info
                            .trace_fn as usize
                    );
                    Some(TraceAt {
                        ptr_to_gc: inner.ptr.as_ptr(),
                        trace_fn: inner.trace_fn,
                    })
                }
                _ => None,
            },
        ));

        while let Some(trace_at) = to_trace.pop() {
            let new_ptr = evac(trace_at, &mut new_nursery, &mut to_trace, marker);
            // Update parrent object's children to point into to-space.
            // The parrent must already be in to-space.
            *(trace_at.ptr_to_gc as *mut *const u8) = new_ptr;
        }
    }

    NURSERY.with(|r| mem::swap(&mut *r.get(), &mut new_nursery));
    assert!(new_nursery.blocks.is_empty());
    POST_BLOCK_COUNT.with(|pbc| pbc.store(BLOCK_COUNT.with(|bc| bc.load(Relaxed)), Relaxed));
}

unsafe fn evac(
    trace_at: TraceAt,
    new_nursery: &mut Blocks,
    to_trace: &mut Vec<TraceAt>,
    marker: bool,
) -> *const u8 {
    let old_ptr = *trace_at.ptr_to_gc;

    let header_ptr = Header::from_ptr(old_ptr as usize) as *mut Header;
    let header = &mut *(header_ptr);
    Header::checksum(header);
    if header.marked != marker {
        // Evac was called on an object in an unmarked block.
        // This means the object was already evacuated.
        return old_ptr;
    }

    assert!((old_ptr as usize - header_ptr as usize) <= BLOCK_SIZE);
    let obj_status = header.evaced.get_mut().entry(old_ptr);
    match obj_status {
        Entry::Vacant(v) => {
            let new_ptr = new_nursery.alloc(header.info);
            ptr::copy_nonoverlapping(old_ptr, new_ptr, header.info.size as usize);
            (trace_at.trace_fn)(new_ptr, to_trace);
            v.insert(ObjectStatus::Moved(new_ptr));

            new_ptr
        }
        Entry::Occupied(mut o) => match o.get_mut() {
            ObjectStatus::Moved(new_ptr) => *new_ptr,
            o @ ObjectStatus::Rooted(_) => {
                let new_ptr = new_nursery.alloc(header.info);
                ptr::copy_nonoverlapping(old_ptr, new_ptr, header.info.size as usize);
                (trace_at.trace_fn)(new_ptr, to_trace);

                let rooted: ObjectStatus = *o;
                *o = ObjectStatus::Moved(new_ptr);
                match rooted {
                    ObjectStatus::Rooted(r) => r.as_ref().ptr.set(new_ptr),
                    _ => unreachable!(),
                }

                // Transfer the Root from old block top new block.
                let new_header = &*Header::from_ptr(new_ptr as usize);
                let new_evaced = &mut *new_header.evaced.get();
                new_evaced.insert(new_ptr, rooted);

                new_ptr
            }
            ObjectStatus::Dropped => panic!(
                "Tried to Evacuate Dropped Object!\n
                 Your GC impl is likely wrong.\n
                 Try setting:\n
                 <{} as GC>::SAFE_TO_DROP = false",
                header.info.type_name
            ),
        },
    }
}
