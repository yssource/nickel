use std::{
    any::type_name,
    cell::UnsafeCell,
    fmt::Debug,
    marker::PhantomData,
    ptr,
    rc::Rc,
    sync::atomic::{AtomicUsize, Ordering::Relaxed}, mem,
};

use gc::Gc;

use crate::blocks::{Blocks, Header};

mod blocks;
mod gc;
mod internals;
#[cfg(test)]
mod tests;

#[derive(Clone)]
pub struct RootStatic<T: 'static + GC> {
    trace_at: Rc<RootAt>,
    _data: PhantomData<T>,
}

#[derive(Clone)]
pub struct Root {
    trace_at: Rc<RootAt>,
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
        dbg!(header);
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
        internals::run_evac()
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

impl<'r, T: GC> From<Gc<'r, T>> for Root {
    fn from(gc: Gc<'r, T>) -> Self {
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
    ptr: AtomicUsize,
    trace_fn: fn(*mut u8, *mut Vec<TraceAt>),
}

impl RootAt {
    pub fn of_val<T: GC>(t: Gc<T>) -> Self {
        let obj_ptr = t.0 as *const T;
        dbg!(obj_ptr);
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

pub trait AsStatic {
    type Static: 'static;
}

/// # Safety
pub unsafe trait GC {
    /// TODO
    /// In the future this can be made const for non DSTs.
    fn trace(_s: &Self, _direct_gc_ptrs: *mut Vec<()>) {}
    /// If this is false we leak.
    /// This uglyness can be avoided in most cases
    /// with a cominations of the aproches I experimented with in sundial-gc.
    /// For Nickel I don't think we need that complexity.
    /// FIXME this should be false, but I don't have time to fix it.
    const SAFE_TO_DROP: bool = true;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcInfo {
    size: u16,
    align: u16,
    needs_drop: bool,
    drop_fn: unsafe fn(*mut u8),
    trace_fn: unsafe fn(*const u8, *mut Vec<RootAt>),
    type_name: &'static str,
}

impl GcInfo {
    pub fn of<T: GC>() -> GcInfo {
        use std::mem;
        GcInfo {
            size: mem::size_of::<T>() as u16,
            align: mem::align_of::<T>() as u16,
            needs_drop: T::SAFE_TO_DROP && mem::needs_drop::<T>(),
            drop_fn: unsafe { mem::transmute(ptr::drop_in_place::<T> as usize) },
            trace_fn: unsafe { mem::transmute(T::trace as usize) },
            type_name: type_name::<T>(),
        }
    }
}

unsafe impl<'g, A: 'g + GC, B: 'g + GC, C: 'g + GC> GC for (A, B, C) {
    fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
        A::trace(&s.0, direct_gc_ptrs);
        B::trace(&s.1, direct_gc_ptrs);
        C::trace(&s.2, direct_gc_ptrs);
    }
}

unsafe impl<'g, A: 'g + GC, B: 'g + GC> GC for (A, B) {
    fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
        A::trace(&s.0, direct_gc_ptrs);
        B::trace(&s.1, direct_gc_ptrs);
    }
}

unsafe impl<'g, A: 'g + GC> GC for (A,) {
    fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
        A::trace(&s.0, direct_gc_ptrs);
    }
}

unsafe impl<T: GC> GC for &'static T {}

unsafe impl GC for usize {}
unsafe impl GC for u128 {}
unsafe impl GC for u64 {}
unsafe impl GC for u32 {}
unsafe impl GC for u16 {}
unsafe impl GC for u8 {}

unsafe impl GC for isize {}
unsafe impl GC for i128 {}
unsafe impl GC for i64 {}
unsafe impl GC for i32 {}
unsafe impl GC for i16 {}
unsafe impl GC for i8 {}

unsafe impl GC for AtomicUsize {}
unsafe impl GC for std::sync::atomic::AtomicIsize {}

unsafe impl GC for String {
    const SAFE_TO_DROP: bool = true;
}

unsafe impl<T: GC> GC for Option<T> {
    fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
        if let Some(t) = s.as_ref() {
            T::trace(t, direct_gc_ptrs)
        }
    }

    const SAFE_TO_DROP: bool = false;
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct GcTypeId(usize);

impl From<&GcInfo> for GcTypeId {
    fn from(info: &GcInfo) -> Self {
        GcTypeId(info.trace_fn as usize)
    }
}

/// This is only safe because the only way to free `Gc`s
/// is with `unsafe fn Root::collect_garbage()`
pub fn gc<T: GC + Debug>(t: T) -> Gc<'static, T> {
    unsafe {
        let gen = Generation::new();
        mem::transmute(gen.gc(t))
    }
}

pub struct Generation {
    nursery: &'static UnsafeCell<Blocks>,
}

impl Generation {
    pub fn new() -> Generation {
        Generation {
            nursery: internals::NURSERY.with(|t| *t),
        }
    }

    pub fn gc<'g, T: GC + Debug + 'g>(&self, t: T) -> Gc<'g, T> {
        unsafe {
            let generation = &mut *self.nursery.get();
            let ptr = generation.alloc(GcInfo::of::<T>()) as *mut T;
            assert!(!ptr.is_null());
            ptr::write(ptr, t);
            Gc::new(&*ptr)
        }
    }

    pub fn from_root<T: GC + Debug>(&self, root: Root) -> Option<Gc<T>> {
        let ptr = root.trace_at.ptr.load(Relaxed);
        let header = unsafe { &*Header::from_ptr(ptr) };
        if header.info == GcInfo::of::<T>() {
            unsafe { Some(Gc::new(&*(ptr as *const T))) }
        } else {
            None
        }
    }

    pub fn try_from_root<T: GC + Debug>(&self, root: Root) -> Result<Gc<T>, String> {
        let ptr = root.trace_at.ptr.load(Relaxed);
        let header = unsafe { &*Header::from_ptr(ptr) };
        if header.info == GcInfo::of::<T>() {
            unsafe { Ok(Gc::new(&*(ptr as *const T))) }
        } else {
            Err(format!(
                "The Root is of type:          `{:?}`\nyou tried to convert it to a: `{}`",
                header,
                type_name::<T>()
            ))
        }
    }

    pub fn from_root_static<T: GC + Debug, S: GC + 'static>(
        &self,
        root: RootStatic<S>,
    ) -> Option<Gc<T>> {
        let ptr = root.trace_at.ptr.load(Relaxed);
        let header = unsafe { &*Header::from_ptr(ptr) };
        if header.info == GcInfo::of::<T>() {
            unsafe { Some(Gc::new(&*(ptr as *const T))) }
        } else {
            None
        }
    }

    pub fn try_from_root_static<T: GC + Debug, S: GC + 'static>(
        &self,
        root: RootStatic<S>,
    ) -> Result<Gc<T>, String> {
        let ptr = root.trace_at.ptr.load(Relaxed);
        let header = unsafe { &*Header::from_ptr(ptr) };
        if header.info == GcInfo::of::<T>() {
            unsafe { Ok(Gc::new(&*(ptr as *const T))) }
        } else {
            Err(format!(
                "The Root is of type:          `{:?}`\nyou tried to convert it to a: `{}`",
                header,
                type_name::<T>()
            ))
        }
    }
}

impl Default for Generation {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Generation {
    fn drop(&mut self) {
        // For now I'm only using static lifetimes and collect will be unsafe.
        // This compromise is necessary to migrate the Nickel code base rapidly.

        let _last_of_gen = unsafe {
            let blocks = &*self.nursery.get();
            blocks.ref_count == 0
        };

        // if !last_of_gen {
        //     return;
        // }

        // let no_roots = internals::ROOTS.with(|t| unsafe { (&*t.get()).is_empty() });

        // if no_roots && last_of_gen {
        //     // Drop the Blocks
        //     unsafe {
        //         Box::from_raw(self.nursery as *const UnsafeCell<Blocks> as *mut UnsafeCell<Blocks>)
        //     };
        // } else {
        //     unsafe { internals::run_evac() }
        // }
    }
}
