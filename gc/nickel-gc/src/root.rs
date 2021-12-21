use std::{
    marker::PhantomData,
    mem,
    rc::Rc,
    sync::atomic::{AtomicUsize, Ordering::Relaxed}, ops::Deref,
};

use crate::{
    blocks::Header,
    gc::{self, Gc},
    internals::{
        self,
        gc_stats::{BLOCK_COUNT, POST_BLOCK_COUNT},
    },
    AsStatic, GC,
};

#[derive(Clone)]
pub struct RootGc<T: 'static + GC> {
    pub(crate) root: Root,
    _data: PhantomData<T>,
}

impl<T: GC + AsStatic> RootGc<T>
where
    T::Static: GC,
{
    pub fn from_gc(gc: Gc<T>) -> RootGc<T::Static> {
        unsafe { mem::transmute(Root::from_gc(gc)) }
    }

    /// This is safe since it gaurenees
    pub fn with<A, F: FnOnce(&T) -> A>(&self, f: F) -> A {
        let t: &T = unsafe { &*(self.root.trace_at.ptr.load(Relaxed) as *const T) };
        f(t)
    }
}

/// This impl is here to help migrate.
/// It's not less safe than the rest currently, but it cannot be made fully safe.
impl<T: GC> Deref for RootGc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*(self.root.trace_at.ptr.load(Relaxed) as *const T) }
    }
}

#[derive(Clone)]
pub struct Root {
    pub(crate) trace_at: Rc<RootAt>,
}

impl Root {
    /// This `Root::from_gc` should be preferred over the `From` impl to aid with inference.
    pub fn from_gc<T: GC>(gc: Gc<T>) -> Root {
        // See `TraceAt` docs for why we ignore the lint.
        #[allow(clippy::mutable_key_type)]
        let roots = internals::ROOTS.with(|roots| unsafe { &mut *roots.get() });
        let trace_at = Rc::new(RootAt::of_val(gc));
        roots.insert(trace_at.clone());

        let header = Header::from_ptr(trace_at.ptr.load(Relaxed));
        // dbg!(header);
        Header::checksum(header);

        Root { trace_at }
    }

    /// This is horribly unsafe!!!
    /// It only exists because migrating from `Rc<T>` to `Gc<'static, T>`
    /// is much simpler than migrating to the safe `Gc<'generation, T>` API.
    ///
    /// # Safety
    ///
    /// This function runs destructors and deallocates memory.
    /// Improper usage will result in use after frees,
    /// segfaults, and every other bad thing you can think of.
    ///
    /// By using this function you must guarantee:
    /// 1. No `Gc<T>`'s exist on this thread, unless they transitively pointed to by a `Root`.
    /// 2. No references to any `Gc`s or their contents exist in this thread.
    pub unsafe fn collect_garbage() {
        if BLOCK_COUNT.load(Relaxed) >= (2 * POST_BLOCK_COUNT.load(Relaxed)) {
            internals::run_evac()
        }
    }


    /// # Safety
    pub unsafe fn try_collect_garbage_other_than<T: GC>(gc: Gc<T>) {
        if BLOCK_COUNT.load(Relaxed) >= (2 * POST_BLOCK_COUNT.load(Relaxed)) {
            let root = Root::from_gc(gc);
            internals::run_evac();
            
        // unsafe { &*(root.trace_at.ptr.load(Relaxed) as *const T) }
        }
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        if Rc::strong_count(&self.trace_at) == 2 {
            // See `TraceAt` docs for why we ignore the lint.
            #[allow(clippy::mutable_key_type)]
            let roots = internals::ROOTS.with(|roots| unsafe { &mut *roots.get() });
            roots.remove(&self.trace_at);
        }
    }
}

impl<'r, T: GC> From<gc::Gc<'r, T>> for Root {
    fn from(gc: gc::Gc<'r, T>) -> Self {
        Root::from_gc(gc)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TraceAt {
    /// A `*const Gc<T>`.
    pub ptr_to_gc: *const *const u8,
    pub trace_fn: fn(*mut u8, *mut Vec<TraceAt>),
}
impl TraceAt {
    pub fn of_val<T: GC>(t: &Gc<T>) -> Self {
        TraceAt {
            ptr_to_gc: t as *const Gc<T> as *const *const u8,
            trace_fn: unsafe { std::mem::transmute(T::trace as usize) },
        }
    }
}

/// It's safe to use `RootAt` as a key,
/// since it's impls ignore it's mutable field `ptr: AtomicUsize`.
/// E.g. `#[allow(clippy::mutable_key_type)]`
#[derive(Debug)]
pub struct RootAt {
    /// `ptr` is a `*const T`
    pub(crate) ptr: AtomicUsize,
    pub(crate) trace_fn: fn(*mut u8, *mut Vec<TraceAt>),
}

impl RootAt {
    pub fn of_val<T: GC>(t: crate::gc::Gc<T>) -> Self {
        let obj_ptr = t.0 as *const T;
        // dbg!(obj_ptr);
        let header = Header::from_ptr(obj_ptr as usize);
        Header::checksum(header);

        RootAt {
            ptr: AtomicUsize::new(obj_ptr as usize),
            trace_fn: unsafe { std::mem::transmute(T::trace as usize) },
        }
    }
}

impl PartialEq for RootAt {
    fn eq(&self, other: &Self) -> bool {
        self.trace_fn as usize == other.trace_fn as usize
    }
}

impl Eq for RootAt {}

impl std::hash::Hash for RootAt {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_usize(self.trace_fn as usize)
    }
}
