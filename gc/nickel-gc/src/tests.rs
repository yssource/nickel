use std::sync::atomic::AtomicIsize;

use crate::internals::gc_stats;

use super::*;

mod nickel_gc {
    #[allow(unused_imports)]
    pub use super::*;
}

use nickel_gc_derive::*;

#[derive(Debug, GC)]
struct List<'g, T> {
    elm: T,
    next: Option<Gc<'g, List<'g, T>>>,
}

impl<'g, T: GC + Clone + Debug> List<'g, T> {
    fn from_vec(gen: &'g Generation, v: &[T]) -> Gc<'g, Self> {
        v.iter()
            .rev()
            .fold(None, |next, elm| Some(List::cons(gen, elm.clone(), next)))
            .unwrap()
    }
}

impl<'r, T: Clone + Debug> From<Gc<'r, List<'r, T>>> for Vec<T> {
    fn from(xs: Gc<'r, List<'r, T>>) -> Self {
        let mut xs = Some(xs);
        let mut v = vec![];
        while let Some(Gc(List { elm, next }, _)) = xs {
            dbg!(xs);
            dbg!(xs.map(|o| o.0 as *const _));
            v.push(elm.clone());
            xs = *next
        }
        v
    }
}

impl<'g, T: GC + Debug> List<'g, T> {
    fn cons(gen: &'g Generation, elm: T, next: Option<Gc<'g, List<'g, T>>>) -> Gc<'g, List<'g, T>> {
        gen.gc(List { elm, next })
    }
}


#[test]
fn lifetimes() {
    let gen = Generation::new();
    let one = gen.gc(1);
    let _two = gen.gc(List {
        elm: one,
        next: Some(List::cons(&gen, one, None)),
    });

    unsafe { Root::collect_garbage() };
}

#[test]
fn alloc() {
    static COUNTED_COUNT: AtomicIsize = AtomicIsize::new(0);
    #[derive(Debug, GC)]
    struct Counted(&'static AtomicIsize);
    impl Drop for Counted {
        fn drop(&mut self) {
            self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    impl Counted {
        fn new() -> Self {
            COUNTED_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Counted(&COUNTED_COUNT)
        }
    }

    let block_count = gc_stats::BLOCK_COUNT.load(std::sync::atomic::Ordering::Relaxed);
    let gen = Generation::new();
    for _ in 0..100_000 {
        // for _ in 0..10 {
        gen.gc((
            1usize,
            2usize,
            (1usize, 1usize, (1usize, 1usize, (1usize, 1usize, 1usize))),
        ));
        gen.gc(Some(1));
        gen.gc(Some(2));
        gen.gc("foo".to_string());
        gen.gc("bar".to_string());
        gen.gc(Counted::new());
    }

    let block_count_1 = gc_stats::BLOCK_COUNT.load(std::sync::atomic::Ordering::Relaxed);

    drop(gen);
    unsafe { Root::collect_garbage() };

    let block_count_2 = gc_stats::BLOCK_COUNT.load(std::sync::atomic::Ordering::Relaxed);
    assert!(block_count < block_count_1);
    assert_eq!(block_count_2, 0);
    assert_eq!(0, COUNTED_COUNT.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn roots() {
    let gen = Generation::new();
    let root1 = Root::from(gen.gc(List::cons(
        &gen,
        "Foo".to_string(),
        Some(List::cons(
            &gen,
            "Bar".to_string(),
            Some(List::cons(&gen, "Bazz".to_string(), None)),
        )),
    )));

    let vec = vec!["Foo".to_string(), "Bar".to_string(), "Bazz".to_string()];
    let root2 = Root::from_gc(List::from_vec(&gen, &vec));

    drop(gen);
    unsafe { Root::collect_garbage() };

    let gen = Generation::new();
    let gc1: Gc<Gc<List<String>>> = gen.from_root(root1.clone()).unwrap();
    let gc2: Gc<List<String>> = gen.try_from_root(root2.clone()).unwrap();

    let vec1: Vec<_> = (*gc1).into();
    let vec2: Vec<_> = gc2.into();
    dbg!(&vec);
    dbg!(&vec1);
    dbg!(&vec2);

    assert_eq!(&vec, &vec1);
    assert_eq!(&vec, &vec2);

    drop(gen);
    unsafe { Root::collect_garbage() };

    let gen = Generation::new();
    let gc1: Gc<Gc<List<String>>> = gen.from_root(root1).unwrap();
    let gc2: Gc<List<String>> = gen.try_from_root(root2).unwrap();

    let vec1: Vec<_> = (*gc1).into();
    let vec2: Vec<_> = gc2.into();
    dbg!(&vec);
    dbg!(&vec1);
    dbg!(&vec2);

    assert_eq!(&vec, &vec1);
    assert_eq!(&vec, &vec2);

    unsafe { Root::collect_garbage() };
}
