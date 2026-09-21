// SPDX-License-Identifier: MPL-2.0
//
// list.rs -- instrusive doubly linked list
// Copyright (C) 2026  Jacob Koziej <jacobkoziej@gmail.com>

#![allow(dead_code)]

use core::cell::UnsafeCell;
use core::marker::{PhantomData, PhantomPinned};
use core::pin::Pin;
use core::ptr;
use core::sync::atomic::AtomicBool;

struct RawNode {
    prev: UnsafeCell<*const RawNode>,
    next: UnsafeCell<*const RawNode>,
    _pin: PhantomPinned,
}

impl RawNode {
    fn init(self: Pin<&Self>) {
        let ptr = ptr::from_ref(self.get_ref());

        unsafe {
            *self.as_ref().prev.get() = ptr;
            *self.as_ref().next.get() = ptr;
        }
    }

    const fn insert(prev: *const RawNode, node: *const RawNode, next: *const RawNode) {
        unsafe {
            *(*prev).next.get() = node;
            *(*node).prev.get() = prev;
            *(*node).next.get() = next;
            *(*next).prev.get() = node;
        }
    }

    fn is_singleton(&self) -> bool {
        let ptr = ptr::from_ref(self);

        let prev = unsafe { &*self.prev.get() };
        let next = unsafe { &*self.next.get() };

        *prev == ptr && *next == ptr
    }

    const fn new() -> Self {
        Self {
            prev: UnsafeCell::new(ptr::null_mut()),
            next: UnsafeCell::new(ptr::null_mut()),
            _pin: PhantomPinned,
        }
    }

    const fn remove(prev: *const RawNode, node: *const RawNode, next: *const RawNode) {
        unsafe {
            *(*prev).next.get() = next;
            *(*next).prev.get() = prev;

            *(*node).next.get() = node;
            *(*node).prev.get() = node;
        }
    }
}

pub trait Role {}

pub struct Node<T, R: Role> {
    raw: RawNode,
    claimed: AtomicBool,
    _marker: PhantomData<fn() -> (T, R)>,
}

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

#[cfg(test)]
mod test {
    use super::*;

    mod raw_node {
        use super::*;
        use core::pin::pin;

        fn ptr(node: Pin<&RawNode>) -> *const RawNode {
            ptr::from_ref(node.get_ref())
        }

        fn ready(node: Pin<&RawNode>) -> Pin<&RawNode> {
            node.init();
            node
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
