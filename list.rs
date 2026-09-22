// SPDX-License-Identifier: MPL-2.0
//
// list.rs -- instrusive doubly linked list
// Copyright (C) 2026  Jacob Koziej <jacobkoziej@gmail.com>

#![allow(dead_code)]

use core::cell::UnsafeCell;
use core::marker::{PhantomData, PhantomPinned};
use core::mem::ManuallyDrop;
use core::ops::{Deref, Drop};
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

struct RawNode {
    prev: UnsafeCell<*const Self>,
    next: UnsafeCell<*const Self>,
    _pin: PhantomPinned,
}

impl RawNode {
    const fn insert(prev: *const Self, node: *const Self, next: *const Self) {
        unsafe {
            *(*prev).next.get() = node;
            *(*node).prev.get() = prev;
            *(*node).next.get() = next;
            *(*next).prev.get() = node;
        }
    }

    const fn is_null(&self) -> bool {
        let prev = unsafe { &*self.prev.get() };
        let next = unsafe { &*self.next.get() };

        prev.is_null() && next.is_null()
    }

    fn is_singleton(&self) -> bool {
        let ptr = ptr::from_ref(self);

        let prev = unsafe { &*self.prev.get() };
        let next = unsafe { &*self.next.get() };

        *prev == ptr && *next == ptr
    }

    const fn new() -> Self {
        Self {
            prev: UnsafeCell::new(ptr::null()),
            next: UnsafeCell::new(ptr::null()),
            _pin: PhantomPinned,
        }
    }

    const fn next(ptr: *const Self) -> *const Self {
        unsafe { *(*ptr).next.get() }
    }

    const fn prev(ptr: *const Self) -> *const Self {
        unsafe { *(*ptr).prev.get() }
    }

    const fn remove(prev: *const Self, node: *const Self, next: *const Self) {
        unsafe {
            *(*prev).next.get() = next;
            *(*next).prev.get() = prev;

            *(*node).next.get() = ptr::null();
            *(*node).prev.get() = ptr::null();
        }
    }
}

pub trait Role {}

pub struct Node<T, R: Role> {
    raw: RawNode,
    claimed: AtomicBool,
    _marker: PhantomData<fn() -> (T, R)>,
}

impl<T, R> Node<T, R>
where
    R: Role,
{
    pub unsafe fn new() -> Self {
        Self {
            raw: RawNode::new(),
            claimed: AtomicBool::new(false),
            _marker: PhantomData,
        }
    }
}

impl<T, R> Deref for Node<T, R>
where
    T: Linked<R>,
    R: Role,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*Self::Target::as_item(ptr::from_ref(self)) }
    }
}

unsafe impl<T, R: Role> Send for Node<T, R> {}
unsafe impl<T, R: Role> Sync for Node<T, R> {}

pub unsafe trait Linked<R: Role>
where
    Self: Sized,
{
    fn as_item(node: *const Node<Self, R>) -> *const Self;
    fn as_node(ptr: *const Self) -> *const Node<Self, R>;
}

#[macro_export]
macro_rules! linked {
    ($ty:ty, $role:ty, $field:ident) => {
        unsafe impl $crate::Linked<$role> for $ty {
            fn as_item(node: *const $crate::Node<Self, $role>) -> *const Self {
                let offset = ::core::mem::offset_of!(Self, $field);

                unsafe { node.byte_sub(offset).cast::<Self>() }
            }

            fn as_node(ptr: *const Self) -> *const $crate::Node<Self, $role> {
                unsafe { &raw const (*ptr).$field }
            }
        }
    };
}

pub struct ListArc<T, R>
where
    T: Linked<R>,
    R: Role,
{
    arc: Arc<T>,
    _marker: PhantomData<fn() -> R>,
}

impl<T, R> ListArc<T, R>
where
    T: Linked<R>,
    R: Role,
{
    unsafe fn from_raw(ptr: *const T) -> Self {
        Self {
            arc: unsafe { Arc::from_raw(ptr) },
            _marker: PhantomData,
        }
    }

    fn into_raw(self) -> *const T {
        let this = ManuallyDrop::new(self);

        Arc::as_ptr(&this.arc)
    }

    pub fn try_from_arc(arc: &Arc<T>) -> Option<Self> {
        let node: &Node<T, R> = unsafe { &*T::as_node(&**arc) };

        node.claimed
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()?;

        Some(Self {
            arc: Arc::clone(arc),
            _marker: PhantomData,
        })
    }
}

impl<T, R> Deref for ListArc<T, R>
where
    T: Linked<R>,
    R: Role,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.arc
    }
}

impl<T, R> Drop for ListArc<T, R>
where
    T: Linked<R>,
    R: Role,
{
    fn drop(&mut self) {
        let node: &Node<T, R> = unsafe { &*T::as_node(&*self.arc) };

        node.claimed.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    struct Foo;
    impl Role for Foo {}

    struct Bar;
    impl Role for Bar {}

    struct Item {
        data: u32,
        foo: Node<Item, Foo>,
        bar: Node<Item, Bar>,
    }

    impl Item {
        fn new(data: u32) -> Self {
            Self {
                data,
                foo: unsafe { Node::<_, Foo>::new() },
                bar: unsafe { Node::<_, Bar>::new() },
            }
        }
    }

    linked! {Item, Foo, foo}
    linked! {Item, Bar, bar}

    const _: () = {
        fn check<T: Send + Sync>() {}
        let _ = check::<Node<Item, Foo>>;
    };

    mod raw_node {
        use super::*;
        use core::pin::{Pin, pin};

        fn ptr(node: Pin<&RawNode>) -> *const RawNode {
            ptr::from_ref(node.get_ref())
        }

        fn prev(node: Pin<&RawNode>) -> *const RawNode {
            unsafe { *node.prev.get() }
        }

        fn next(node: Pin<&RawNode>) -> *const RawNode {
            unsafe { *node.next.get() }
        }

        fn assert_ring(nodes: &[Pin<&RawNode>]) {
            let n = nodes.len();

            for i in 0..n {
                let next = ptr(nodes[(i + 1) % n]);
                let prev = ptr(nodes[(i + n - 1) % n]);

                assert_eq!(self::next(nodes[i]), next);
                assert_eq!(self::prev(nodes[i]), prev);
            }
        }

        #[test]
        fn insert() {
            let a = pin!(RawNode::new());
            let a = a.into_ref();
            let b = pin!(RawNode::new());
            let b = b.into_ref();

            RawNode::insert(ptr(b), ptr(a), ptr(b));
            assert_ring(&[a, b]);

            let c = pin!(RawNode::new());
            let c = c.into_ref();

            RawNode::insert(ptr(b), ptr(c), ptr(a));
            assert_ring(&[a, b, c]);
        }

        #[test]
        fn if_null() {
            let node = pin!(RawNode::new());
            let node = node.into_ref();

            assert!(node.is_null());
        }

        #[test]
        fn is_singleton() {
            let node = pin!(RawNode::new());
            let node = node.into_ref();

            assert!(!node.is_singleton());

            RawNode::insert(ptr(node), ptr(node), ptr(node));

            assert!(node.is_singleton());
        }

        #[test]
        fn remove() {
            let a = pin!(RawNode::new());
            let a = a.into_ref();
            let b = pin!(RawNode::new());
            let b = b.into_ref();
            let c = pin!(RawNode::new());
            let c = c.into_ref();

            RawNode::insert(ptr(b), ptr(a), ptr(b));
            RawNode::insert(ptr(b), ptr(c), ptr(a));

            RawNode::remove(ptr(b), ptr(c), ptr(a));

            assert!(c.is_null());
            assert_ring(&[a, b]);

            RawNode::remove(ptr(b), ptr(a), ptr(b));

            assert!(a.is_null());
            assert!(b.is_singleton());

            RawNode::remove(ptr(b), ptr(b), ptr(b));

            assert!(b.is_null());
        }
    }

    mod list_arc {
        use super::*;

        #[test]
        fn only_mints_for_unclaimed() {
            let arc = Arc::new(Item::new(0));

            let foo = ListArc::<_, Foo>::try_from_arc(&arc);

            assert!(foo.is_some());

            assert!(ListArc::<_, Foo>::try_from_arc(&arc).is_none());
        }

        #[test]
        fn into_raw_skips_drop() {
            let arc = Arc::new(Item::new(0));
            let foo = ListArc::<_, Foo>::try_from_arc(&arc).unwrap();

            assert_eq!(Arc::strong_count(&arc), 2);

            let ptr = ListArc::into_raw(foo);

            assert_eq!(Arc::strong_count(&arc), 2);
            assert!(ListArc::<_, Foo>::try_from_arc(&arc).is_none());

            let foo = unsafe { ListArc::<_, Foo>::from_raw(ptr) };

            assert_eq!(Arc::strong_count(&arc), 2);

            drop(foo);

            assert_eq!(Arc::strong_count(&arc), 1);
        }

        #[test]
        fn drop_releases_claim() {
            let arc = Arc::new(Item::new(0));

            let foo = ListArc::<_, Foo>::try_from_arc(&arc).unwrap();

            drop(foo);

            assert!(ListArc::<_, Foo>::try_from_arc(&arc).is_some());
        }

        #[test]
        fn deref_item() {
            let arc = Arc::new(Item::new(1));

            let foo = ListArc::<_, Foo>::try_from_arc(&arc).unwrap();

            assert_eq!(foo.data, 1);
            assert!(ptr::eq(arc.as_ref(), &*foo));
        }
    }
}
