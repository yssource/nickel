use std::{ops::Deref, borrow::Borrow, fmt::Debug};

use crate::{GC, root::TraceAt};


#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub struct Gc<'g, T>(pub &'g T, pub P);

impl<'g, T> Gc<'g, T> {
    /// # Safety
    /// You should never construct a `Gc`.
    /// `P` exists to allow destructuring, but not construction.
    #[inline(always)]
    pub unsafe fn new(t: &'g T) -> Self {
        Gc(t, P(()))
    }
}

unsafe impl<'g, T: GC> GC for Gc<'g, T> {
    fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
        // TODO this seems like it could cause issues, since `GC: Copy`.
        // Fix it by replacing `&self` with `*const Self`
        unsafe { &mut *(direct_gc_ptrs as *mut Vec<TraceAt>) }.push(TraceAt::of_val(s))
    }

    const SAFE_TO_DROP: bool = true;
}

/// Just here to prevent construction of `Gc` & `Box`.
/// Use `_` to pattern match against `Gc` & `Box`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct P(());

impl<'r, T> Copy for Gc<'r, T> {}

impl<'r, T> Clone for Gc<'r, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'r, T> Deref for Gc<'r, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0
    }
}

impl<'r, T> AsRef<T> for Gc<'r, T> {
    fn as_ref(&self) -> &T {
        self.0
    }
}

impl<'r, T> Borrow<T> for Gc<'r, T> {
    fn borrow(&self) -> &T {
        self.0
    }
}

impl<'r, T: Debug> Debug for Gc<'r, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Gc").field(self.0).finish()
    }
}
