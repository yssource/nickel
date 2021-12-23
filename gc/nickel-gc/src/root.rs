use std::{
    any::type_name, cell::Cell, fmt::Debug, marker::PhantomData, mem, ops::Deref, ptr::NonNull,
};

use crate::{
    blocks::Header,
    gc::{self, Gc},
    generation::Generation,
    internals::{self, gc_stats},
    AsStatic, GcInfo, GC,
};

#[derive(Clone)]
pub struct RootGc<T: GC> {
    pub(crate) root: Root,
    _data: PhantomData<T>,
}

impl<T: GC> RootGc<T> {
    pub fn new(t: T) -> Self {
        let gen = Generation::new();
        RootGc::from_gc(gen.gc(t))
    }

    pub fn from_gc<O: GC>(gc: Gc<T>) -> RootGc<O> {
        if type_name::<O>() != type_name::<T>() {
            panic!("Miss matched types in from_gc")
        }
        unsafe { mem::transmute(Root::from_gc(gc)) }
    }

    pub fn as_ptr(root: &RootGc<T>) -> *const T {
        root.deref()
    }

    pub fn get_mut(root: &mut RootGc<T>) -> Option<&mut T> {
        let inner = unsafe { root.root.inner.as_ref() };
        if inner.ref_count.get() == 1 {
            Some(unsafe { &mut *(inner.ptr.get() as *mut u8 as *mut T) })
        } else {
            None
        }
    }

    pub fn make_mut(root: &mut RootGc<T>) -> &mut T
    where
        T: Clone,
    {
        let inner = unsafe { root.root.inner.as_ref() };
        if inner.ref_count.get() == 1 {
            unsafe { &mut *(inner.ptr.get() as *mut u8 as *mut T) }
        } else {
            *root = RootGc::new((**root).clone());

            let inner = unsafe { root.root.inner.as_ref() };
            unsafe { &mut *(inner.ptr.get() as *mut u8 as *mut T) }
        }
    }
}

impl<T: GC + PartialEq> PartialEq for RootGc<T> {
    fn eq(&self, other: &Self) -> bool {
        self.deref().eq(other.deref())
    }
}
impl<T: GC + Eq> Eq for RootGc<T> {}

impl<T: GC + PartialOrd> PartialOrd for RootGc<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.deref().partial_cmp(other.deref())
    }
}

impl<T: GC + Ord> Ord for RootGc<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.deref().cmp(other.deref())
    }
}

impl<T: GC + Debug> Debug for RootGc<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(f)
    }
}

impl<T: GC + Default> Default for RootGc<T> {
    fn default() -> Self {
        RootGc::new(T::default())
    }
}

unsafe impl<T: GC> GC for RootGc<T> {
    unsafe fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
        Root::trace(&s.root, direct_gc_ptrs)
    }
    const SAFE_TO_DROP: bool = true;
}

impl<T: GC + AsStatic> AsStatic for RootGc<T>
where
    T::Static: GC,
{
    type Static = RootGc<T::Static>;
}

/// This impl is here to help migrate.
/// It's not less safe than the rest of the API currently, but it cannot ever be made fully safe.
impl<T: GC> Deref for RootGc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*((self.root.inner.as_ref()).ptr.get() as *const T) }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Root {
    /// Constructing a Root is unsafe.
    /// FIXME make private
    pub(crate) inner: NonNull<RootInner>,
}

impl Root {
    /// This `Root::from_gc` should be preferred over the `From` impl to aid with inference.
    pub fn from_gc<T: GC>(gc: Gc<T>) -> Root {
        let roots = unsafe { &mut *Header::from_gc(gc).evaced.get() };
        let obj_status = roots
            .entry(gc.0 as *const T as *const u8)
            .or_insert_with(|| {
                ObjectStatus::Rooted(NonNull::from(Box::leak(Box::new(RootInner::new(gc)))))
            });

        let inner = match obj_status {
            ObjectStatus::Rooted(r) => *r,
            e => panic!("Attempted to root a object with existing status: {:?}", e),
        };
        unsafe {
            inner
                .as_ref()
                .ref_count
                .set(inner.as_ref().ref_count.get() + 1)
        }

        Root { inner }
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

    pub fn maybe_collect_garbage() {
        if gc_stats::thread_block_count() > 2 * gc_stats::thread_post_block_count() {
            unsafe { internals::run_evac() }
        }
    }
}

impl AsStatic for Root {
    type Static = Self;
}

unsafe impl GC for Root {
    unsafe fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
        let inner = s.inner.as_ref();
        let ptr = inner.ptr.get();

        let traced_count = if inner.collection_marker.get() == internals::marker() {
            let traced_count = inner.traced_count.get();
            inner.traced_count.set(traced_count + 1);
            traced_count
        } else {
            inner.collection_marker.set(internals::marker());
            inner.traced_count.set(1);
            1
        };

        let ref_count = inner.ref_count.get();
        if traced_count == ref_count {
            // All `Root`s live in the GC heap.
            // Hence we can now demote them to a ordinary `Gc`
            let header = &*Header::from_ptr(ptr as usize);
            let evaced = &mut *header.evaced.get();
            evaced.remove(&ptr);
            // Box::from_raw(inner as *const _ as *mut RootInner);
        };
        let direct_gc_ptrs = mem::transmute::<_, *mut Vec<TraceAt>>(direct_gc_ptrs);
        { &mut *(direct_gc_ptrs as *mut Vec<TraceAt>) }.push(TraceAt {
            ptr_to_gc: inner.ptr.as_ptr(),
            trace_fn: inner.trace_fn,
        })
    }
    const SAFE_TO_DROP: bool = true;
}

impl Clone for Root {
    fn clone(&self) -> Self {
        let inner = unsafe { self.inner.as_ref() };
        let ref_count = inner.ref_count.get();
        inner.ref_count.set(ref_count + 1);

        Root { inner: self.inner }
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let inner = unsafe { self.inner.as_ref() };
        let ref_count = inner.ref_count.get();
        inner.ref_count.set(ref_count - 1);
        if ref_count == 1 {
            let ptr = inner.ptr.get();
            unsafe {
                let header = &*Header::from_ptr(ptr as usize);
                let evaced = &mut *header.evaced.get();
                evaced.remove(&ptr);
                Box::from_raw(inner as *const _ as *mut RootInner);
            }
        };
        // Running destructors is handled by the Underlying Gc, not Root.
        // TODO add debug assertions
    }
}

impl<'r, T: GC> From<gc::Gc<'r, T>> for Root {
    fn from(gc: gc::Gc<'r, T>) -> Self {
        Root::from_gc(gc)
    }
}

impl<T: 'static + GC> From<RootGc<T>> for Root {
    fn from(root: RootGc<T>) -> Self {
        root.root
    }
}

impl<T: 'static + GC> TryFrom<Root> for RootGc<T> {
    type Error = String;

    fn try_from(root: Root) -> Result<Self, Self::Error> {
        let ptr = unsafe { root.inner.as_ref() }.ptr.get();
        let header = unsafe { &*Header::from_ptr(ptr as usize) };
        if header.info == GcInfo::of::<T>() {
            Ok(RootGc {
                root,
                _data: PhantomData,
            })
        } else {
            Err(format!(
                "The Root is of type:          `{:?}`\nyou tried to convert it to a: `{}`",
                header,
                type_name::<T>()
            ))
        }
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
///
/// This is like a Rc, but it handles cycles.
///
/// TODO make !Send, and !Sync
/// See if UnsafeCell is any faster.
/// For now I'm using Atomics with Relaxed ordering because it's simpler.
#[derive(Debug)]
pub struct RootInner {
    /// `ptr` is a `*const T`
    pub(crate) ptr: Cell<*const u8>,
    pub(crate) trace_fn: fn(*mut u8, *mut Vec<TraceAt>),
    // drop_fn: unsafe fn(*mut u8),
    /// The marker of the collection phase asscoated with the traced_count.
    /// Right now it's just a two space collector, hence bool.
    collection_marker: Cell<bool>,
    /// The number of references evacuated durring a collection phase.
    traced_count: Cell<usize>,
    /// This is the count of all owning references.
    /// ref_count >= traced_count
    ref_count: Cell<usize>,
}

impl RootInner {
    fn new<T: GC>(t: crate::gc::Gc<T>) -> Self {
        let obj_ptr = t.0 as *const T;
        let header = Header::from_ptr(obj_ptr as usize);
        Header::checksum(header);

        RootInner {
            ptr: Cell::from(obj_ptr as *const u8),
            trace_fn: unsafe { std::mem::transmute(T::trace as usize) },
            // drop_fn: unsafe { mem::transmute(ptr::drop_in_place::<T> as usize) },
            collection_marker: Cell::from(internals::marker()),
            traced_count: Cell::from(0),
            ref_count: Cell::from(0),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub enum ObjectStatus {
    /// The object was moved to the pointer.
    Moved(*const u8),
    /// The object is rooted.
    /// `RootInner.ptr` always points to the current location of the object.
    /// If `RootInner.ptr` is in this `Block` the object has yet to be evacuated.
    Rooted(NonNull<RootInner>),
    /// The object's destructor has been run.
    /// This is only needed for types that are not marked safe to drop.
    Dropped,
}
