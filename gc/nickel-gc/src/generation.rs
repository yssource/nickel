use std::any::type_name;
use std::{cell::UnsafeCell, ptr};
use std::fmt::Debug;
use std::sync::atomic::Ordering::Relaxed;

use crate::GcInfo;
use crate::blocks::Header;
use crate::root::{Root, RootStatic};
use crate::{blocks::Blocks, internals, gc::Gc, GC};


pub struct Generation {
    nursery: &'static UnsafeCell<Blocks>,
}

impl Generation {
    pub fn new() -> Generation {
        Generation {
            nursery: internals::NURSERY.with(|t| *t),
        }
    }

    pub fn gc<'g, T: GC + 'g>(&self, t: T) -> Gc<'g, T> {
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
