use std::{
    alloc::{self, Layout},
    cell::UnsafeCell,
    collections::HashMap,
    mem::transmute,
    ops::{Deref, DerefMut},
    ptr::{self, drop_in_place},
    sync::atomic,
};

use crate::{
    internals::{self, gc_stats},
    root::ObjectStatus,
};

use super::{GcInfo, GcTypeId};

pub const HEADER_ALIGNMENT_BITS: usize = 13;
pub const BLOCK_SIZE: usize = 1 << HEADER_ALIGNMENT_BITS;
pub const BLOCK_LAYOUT: Layout = match Layout::from_size_align(BLOCK_SIZE, BLOCK_SIZE) {
    Ok(l) => l,
    Err(_) => panic!("Could not make constant BLOCK_LAYOUT"),
};

#[derive(Debug)]
pub struct Header {
    pub info: GcInfo,
    pub marked: bool,
    /// Make count debug only.
    count: u16,
    bottom: *const u8,
    current: *const u8,
    /// This could be way faster.
    pub evaced: UnsafeCell<HashMap<*const u8, crate::root::ObjectStatus>>,
    /// Make debug only;
    pub checksum: [u64; 8],
}

impl Header {
    /// Random numbers used to check the integrity of a header.
    /// TODO Make debug only
    const CHECKSUM: [u64; 8] = [
        5255335, 60229479, 26201331, 85993136, 57558466, 84202187, 16547791, 78315812,
    ];
    pub unsafe fn new_raw(info: GcInfo) -> *mut Header {
        let size: usize = info.size.into();
        let align: usize = info.align.into();
        assert!(align.is_power_of_two());
        assert!(align <= u16::MAX.into());
        assert!(size <= u16::MAX.into());

        let ptr = alloc::alloc(BLOCK_LAYOUT);
        if ptr.is_null() {
            panic!("Could Not Allocate Block for GC!")
        };
        let ptr = ptr as usize;

        let bottom = (ptr as *const Header).offset(1) as usize;
        let bottom = (bottom & !(align - 1)) + (size * 10);

        assert!(bottom > ptr);
        assert!((bottom) < ptr + BLOCK_SIZE);

        let count = ((BLOCK_SIZE - (bottom - ptr)) / size) - 1;
        assert!(count > 0);
        assert!(count <= (BLOCK_SIZE - (bottom - ptr)) / size);

        let current = bottom + (size * count);
        assert!(current <= BLOCK_SIZE + ptr);

        let header_ptr = ptr as *mut Header;
        ptr::write(
            header_ptr,
            Header {
                info,
                marked: internals::MARKER.with(|m| *m.get()),
                count: count as u16,
                bottom: bottom as *const u8,
                current: current as *const u8,
                evaced: UnsafeCell::new(HashMap::default()),
                checksum: Self::CHECKSUM,
            },
        );

        gc_stats::BLOCK_COUNT.fetch_add(1, atomic::Ordering::Relaxed);

        // dbg!(header_ptr);
        header_ptr
    }

    /// Will return null if block is out of space.
    pub fn alloc(&mut self) -> *const u8 {
        let current = (self.current as usize).saturating_sub(self.info.size as usize) as *const u8;
        assert!(self.bottom as usize > 0);
        if current > self.bottom {
            self.count -= 1;
            self.current = current;
            current as *mut u8
        } else {
            ptr::null_mut()
        }
    }

    pub fn from_ptr(ptr: usize) -> *const Header {
        let offset = ptr & (BLOCK_SIZE - 1);
        (ptr - offset) as _
    }

    pub(crate) fn from_gc<T>(gc: crate::gc::Gc<T>) -> &Header {
        unsafe { &*Header::from_ptr(gc.0 as *const T as usize) }
    }

    pub fn checksum(header: *const Header) {
        assert!(unsafe { &*header }.checksum == Self::CHECKSUM);
    }
}

#[derive(PartialEq, Eq)]
pub struct HeaderRef(*mut Header);

impl HeaderRef {
    pub fn new(info: GcInfo) -> HeaderRef {
        HeaderRef(unsafe { Header::new_raw(info) })
    }
}

impl Deref for HeaderRef {
    type Target = Header;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.0 }
    }
}
impl DerefMut for HeaderRef {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.0 }
    }
}

impl Drop for HeaderRef {
    fn drop(&mut self) {
        if self.info.needs_drop {
            let size = self.info.size as usize;
            let ptr = self.0 as usize;
            let current = self.current as usize;
            let bottom = self.bottom as usize;
            let total_count = ((BLOCK_SIZE - (bottom - ptr)) / size) - 1;
            let alloc_count = ((BLOCK_SIZE - (current - ptr)) / size) - 1;

            for i in (total_count - alloc_count)..total_count {
                let ptr = (bottom + (size * i)) as *const u8;
                let evaced = unsafe { &mut *self.evaced.get() };
                if let std::collections::hash_map::Entry::Occupied(mut e) = evaced.entry(ptr) {
                    unsafe {
                        (self.info.drop_fn)(ptr as *mut u8);
                    }
                    // This is only needed for types that are not marked safe to drop.
                    // However it is also a nice correctness check.
                    e.insert(ObjectStatus::Dropped);
                }
            }
        }

        unsafe {
            drop_in_place(&mut self.evaced);
            alloc::dealloc(self.0 as *mut u8, BLOCK_LAYOUT);
        }

        gc_stats::BLOCK_COUNT.fetch_sub(1, atomic::Ordering::Relaxed);
    }
}

#[derive(PartialEq, Eq, Default)]
pub struct Blocks {
    /// u8 is realy a `T` determined by `GcTypeId`.
    pub blocks: HashMap<GcTypeId, Vec<HeaderRef>>,
    pub ref_count: usize,
}

impl Blocks {
    pub fn header_mut(&mut self, info: GcInfo) -> &mut Vec<HeaderRef> {
        let header_ptrs = self
            .blocks
            .entry((&info).into())
            .or_insert_with(|| vec![HeaderRef::new(info)]);

        unsafe { transmute(header_ptrs) }
    }

    pub fn alloc(&mut self, info: GcInfo) -> *mut u8 {
        let headers = self.header_mut(info);
        if headers.is_empty() {
            headers.push(HeaderRef::new(info));
        };

        let ptr = headers.last_mut().unwrap().alloc();
        if !ptr.is_null() {
            ptr as *mut u8
        } else {
            headers.push(HeaderRef::new(info));
            let header = headers.last_mut().unwrap();
            Header::checksum(header.0);
            let ptr = header.alloc();
            Header::checksum(header.0);

            if !ptr.is_null() {
                ptr as *mut u8
            } else {
                panic!("Could Not Allocate Header!")
            }
        }
    }
}
