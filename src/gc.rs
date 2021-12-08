use std::{borrow::Borrow, fmt::Debug, ops::Deref};

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub struct Gc<'r, T>(pub &'r T, pub P);

impl<'r, T> Gc<'r, T> {
    #[inline(always)]
    pub unsafe fn new(t: &'r T) -> Self {
        Gc(t, P(()))
    }
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

pub struct Evac (*mut u8, fn(*mut u8) -> Vec<Evac>);

pub unsafe trait GC {
    fn evacuate(gen: Generation, obj_of_self: *mut u8, &mut );
}


