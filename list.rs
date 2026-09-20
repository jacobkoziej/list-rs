// SPDX-License-Identifier: MPL-2.0
//
// list.rs -- instrusive doubly linked list
// Copyright (C) 2026  Jacob Koziej <jacobkoziej@gmail.com>

use core::cell::UnsafeCell;
use core::marker::{PhantomData, PhantomPinned};
use core::pin::Pin;
use core::ptr::NonNull;
use core::sync::atomic::AtomicBool;

type RawNodePtr = UnsafeCell<NonNull<RawNode>>;

struct RawNode {
    prev: RawNodePtr,
    next: RawNodePtr,
    _pin: PhantomPinned,
}

impl RawNode {
    fn init(self: Pin<&Self>) {
        let ptr = NonNull::from(self.get_ref());

        unsafe {
            *self.as_ref().prev.get() = ptr;
            *self.as_ref().next.get() = ptr;
        }
    }

    fn insert(prev: NonNull<RawNode>, node: NonNull<RawNode>, next: NonNull<RawNode>) {
        unsafe {
            *prev.as_ref().next.get() = node;
            *node.as_ref().prev.get() = prev;
            *node.as_ref().next.get() = next;
            *next.as_ref().prev.get() = node;
        }
    }

    fn is_singleton(&self) -> bool {
        let ptr = NonNull::from(self);
        let prev = unsafe { &*self.prev.get() };
        let next = unsafe { &*self.next.get() };

        *prev == ptr && *next == ptr
    }

    fn new() -> Self {
        Self {
            prev: UnsafeCell::new(NonNull::dangling()),
            next: UnsafeCell::new(NonNull::dangling()),
            _pin: PhantomPinned,
        }
    }

    fn remove(prev: NonNull<RawNode>, node: NonNull<RawNode>, next: NonNull<RawNode>) {
        unsafe {
            *prev.as_ref().next.get() = next;
            *next.as_ref().prev.get() = prev;

            *node.as_ref().next.get() = node;
            *node.as_ref().prev.get() = node;
        }
    }
}

pub trait Role {}

pub struct Node<T, R: Role> {
    raw: RawNode,
    claimed: AtomicBool,
    _marker: PhantomData<fn() -> (T, R)>,
}

#[cfg(test)]
mod test {
    use super::*;

    mod raw_node {
        use super::*;
        use core::pin::pin;

        fn ptr(node: Pin<&RawNode>) -> NonNull<RawNode> {
            NonNull::from(node.get_ref())
        }

        fn ready(node: Pin<&RawNode>) -> Pin<&RawNode> {
            node.init();
            node
        }

        fn prev(node: Pin<&RawNode>) -> NonNull<RawNode> {
            unsafe { *node.prev.get() }
        }

        fn next(node: Pin<&RawNode>) -> NonNull<RawNode> {
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
            let b = pin!(RawNode::new());

            let a = ready(a.as_ref());
            let b = ready(b.as_ref());

            RawNode::insert(ptr(b), ptr(a), ptr(b));
            assert_ring(&[a, b]);

            let c = pin!(RawNode::new());

            let c = ready(c.as_ref());

            RawNode::insert(ptr(b), ptr(c), ptr(a));
            assert_ring(&[a, b, c]);
        }

        #[test]
        fn is_singleton() {
            let node = pin!(RawNode::new());

            assert!(!node.is_singleton());

            node.as_ref().init();

            assert!(node.is_singleton());
        }

        #[test]
        fn remove() {
            let a = pin!(RawNode::new());
            let b = pin!(RawNode::new());
            let c = pin!(RawNode::new());

            let a = ready(a.as_ref());
            let b = ready(b.as_ref());
            let c = ready(c.as_ref());

            RawNode::insert(ptr(b), ptr(a), ptr(b));
            RawNode::insert(ptr(b), ptr(c), ptr(a));

            RawNode::remove(ptr(b), ptr(c), ptr(a));

            assert!(c.is_singleton());
            assert_ring(&[a, b]);

            RawNode::remove(ptr(b), ptr(a), ptr(b));

            assert!(a.is_singleton());
            assert!(b.is_singleton());
        }
    }
}
