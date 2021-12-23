use std::{
    any::type_name, borrow::Borrow, cell::RefCell, fmt::Debug, mem, ops::Deref, ptr, rc::Rc,
    sync::atomic::AtomicUsize,
};

use root::{RootInner, TraceAt};

mod blocks;
pub mod gc;
pub mod generation;
mod internals;
pub mod root;
#[cfg(test)]
mod tests;

pub struct Gc<T>(*const T);

impl<T: GC> Gc<T> {
    pub fn new(t: T) -> Self {
        gc(t)
    }

    pub fn as_ptr(self) -> *const T {
        self.0
    }
}

impl<'a, T> From<Gc<T>> for gc::Gc<'a, T> {
    fn from(a: Gc<T>) -> Self {
        unsafe { mem::transmute(a) }
    }
}

impl<'a, T> From<gc::Gc<'a, T>> for Gc<T> {
    fn from(a: gc::Gc<'a, T>) -> Self {
        unsafe { mem::transmute(a) }
    }
}

impl<T: AsStatic> AsStatic for Gc<T> {
    type Static = Gc<T::Static>;
}

unsafe impl<'g, T: GC> GC for Gc<T> {
    unsafe fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
        (&mut *(direct_gc_ptrs as *mut Vec<TraceAt>)).push(TraceAt::of_val(&((*s).into())))
    }

    const SAFE_TO_DROP: bool = true;
}

impl<T> Clone for Gc<T> {
    fn clone(&self) -> Self {
        Gc(self.0)
    }
}

impl<T> Copy for Gc<T> {}

impl<T> Deref for Gc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.0 }
    }
}

impl<T> AsRef<T> for Gc<T> {
    fn as_ref(&self) -> &T {
        self.deref()
    }
}

impl<T> Borrow<T> for Gc<T> {
    fn borrow(&self) -> &T {
        self.deref()
    }
}

impl<T: PartialEq> PartialEq for Gc<T> {
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}
impl<T: Eq> Eq for Gc<T> {}

impl<T: PartialOrd> PartialOrd for Gc<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.deref().partial_cmp(other.deref())
    }
}

impl<T: Ord> Ord for Gc<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.deref().cmp(other.deref())
    }
}

impl<T: Debug> Debug for Gc<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(f)
    }
}

impl<T: Default + GC> Default for Gc<T> {
    fn default() -> Self {
        gc(T::default())
    }
}

pub trait AsStatic {
    type Static: 'static;
}

/// # Safety
/// Derive this.
pub unsafe trait GC {
    /// In the future this can be made const for non DSTs.
    /// # Safety
    /// Don't implement this use the derive macro.
    /// If you must implement this just call `trace` on each feild under your type.
    unsafe fn trace(_s: &Self, _direct_gc_ptrs: *mut Vec<()>) {}
    /// If this is false we leak.
    /// This uglyness can be avoided in most cases
    /// with a cominations of the aproches I experimented with in sundial-gc.
    /// For Nickel I don't think we need that complexity.
    /// FIXME this should be false, but I don't have time to fix it.
    const SAFE_TO_DROP: bool = true;
    const GC_COUNT: u16 = 1;
}

#[macro_export]
/// # Saftey
/// Are you sure this type does not transitively contain any `Gc`s?
macro_rules! unsafe_impl_gc_static {
    ($ty:ty) => {
        unsafe impl nickel_gc::GC for $ty
        where
            $ty: 'static,
        {
            const SAFE_TO_DROP: bool = true;
            const GC_COUNT: u16 = 0;
        }

        impl nickel_gc::AsStatic for $ty {
            type Static = Self;
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcInfo {
    size: u16,
    align: u16,
    needs_drop: bool,
    drop_fn: unsafe fn(*mut u8),
    trace_fn: unsafe fn(*const u8, *mut Vec<RootInner>),
    type_name: &'static str,
}

impl GcInfo {
    pub fn of<T: GC>() -> GcInfo {
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
    unsafe fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
        A::trace(&s.0, direct_gc_ptrs);
        B::trace(&s.1, direct_gc_ptrs);
        C::trace(&s.2, direct_gc_ptrs);
    }
}

unsafe impl<'g, A: 'g + GC, B: 'g + GC> GC for (A, B) {
    unsafe fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
        A::trace(&s.0, direct_gc_ptrs);
        B::trace(&s.1, direct_gc_ptrs);
    }
}

unsafe impl<'g, A: 'g + GC> GC for (A,) {
    unsafe fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
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

unsafe impl GC for f64 {}
unsafe impl GC for f32 {}

unsafe impl GC for bool {}
unsafe impl GC for char {}

unsafe impl GC for AtomicUsize {}
unsafe impl GC for std::sync::atomic::AtomicIsize {}

unsafe impl GC for String {
    const SAFE_TO_DROP: bool = true;
}

unsafe impl GC for std::ffi::OsString {
    const SAFE_TO_DROP: bool = true;
}

unsafe impl<T: GC> GC for Option<T> {
    unsafe fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
        if let Some(t) = s.as_ref() {
            T::trace(t, direct_gc_ptrs)
        }
    }
}

unsafe impl<T: GC> GC for std::cell::RefCell<T> {
    unsafe fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
        let t = s.borrow();
        T::trace(&t, direct_gc_ptrs)
    }
}

unsafe impl<T: GC> GC for Rc<T> {
    unsafe fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
        let t = s.deref();
        T::trace(t, direct_gc_ptrs)
    }
}

unsafe impl<T: GC> GC for Box<T> {
    unsafe fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
        let t = s.deref();
        T::trace(t, direct_gc_ptrs)
    }
}

unsafe impl<T: GC> GC for Vec<T> {
    unsafe fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
        s.iter().for_each(|t| T::trace(t, direct_gc_ptrs))
    }
}

unsafe impl<K: GC, V: GC> GC for std::collections::HashMap<K, V> {
    unsafe fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
        s.iter().for_each(|(k, v)| {
            K::trace(k, direct_gc_ptrs);
            V::trace(v, direct_gc_ptrs);
        })
    }
}

impl<'g, A: 'g + AsStatic, B: 'g + AsStatic, C: 'g + AsStatic> AsStatic for (A, B, C) {
    type Static = (A::Static, B::Static, C::Static);
}

impl<'g, A: 'g + AsStatic, B: 'g + AsStatic> AsStatic for (A, B) {
    type Static = (A::Static, B::Static);
}

impl<'g, A: 'g + AsStatic> AsStatic for (A,) {
    type Static = (A::Static,);
}

impl<T: AsStatic> AsStatic for &'static T {
    type Static = Self;
}

impl AsStatic for usize {
    type Static = usize;
}
impl AsStatic for u128 {
    type Static = u128;
}
impl AsStatic for u64 {
    type Static = u64;
}
impl AsStatic for u32 {
    type Static = u32;
}
impl AsStatic for u16 {
    type Static = u16;
}
impl AsStatic for u8 {
    type Static = u8;
}

impl AsStatic for isize {
    type Static = isize;
}
impl AsStatic for i128 {
    type Static = i128;
}
impl AsStatic for i64 {
    type Static = i64;
}
impl AsStatic for i32 {
    type Static = i32;
}
impl AsStatic for i16 {
    type Static = i16;
}
impl AsStatic for i8 {
    type Static = i8;
}

impl AsStatic for f64 {
    type Static = f64;
}
impl AsStatic for f32 {
    type Static = f32;
}

impl AsStatic for bool {
    type Static = bool;
}
impl AsStatic for char {
    type Static = char;
}

impl AsStatic for AtomicUsize {
    type Static = AtomicUsize;
}
impl AsStatic for std::sync::atomic::AtomicIsize {
    type Static = Self;
}

impl AsStatic for String {
    type Static = Self;
}

impl AsStatic for std::ffi::OsString {
    type Static = Self;
}

impl<T: AsStatic> AsStatic for Option<T> {
    type Static = Option<T::Static>;
}

impl<T: AsStatic> AsStatic for std::cell::RefCell<T> {
    type Static = RefCell<T::Static>;
}

impl<T: AsStatic> AsStatic for Rc<T> {
    type Static = Rc<T::Static>;
}

impl<T: AsStatic> AsStatic for Box<T> {
    type Static = Box<T::Static>;
}

impl<T: AsStatic> AsStatic for Vec<T> {
    type Static = Vec<T::Static>;
}

impl<K: AsStatic, V: AsStatic> AsStatic for std::collections::HashMap<K, V> {
    type Static = std::collections::HashMap<K::Static, V::Static>;
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
pub fn gc<T: GC>(t: T) -> Gc<T> {
    unsafe {
        let gen = generation::Generation::new();
        mem::transmute(gen.gc(t))
    }
}
