// SPDX-License-Identifier: MPL-2.0
//
// list.rs -- instrusive doubly linked list
// Copyright (C) 2026  Jacob Koziej <jacobkoziej@gmail.com>

use core::cell::UnsafeCell;
use core::marker::PhantomPinned;
use core::ptr::NonNull;

type RawNodePtr = UnsafeCell<NonNull<RawNode>>;

struct RawNode {
    prev: RawNodePtr,
    next: RawNodePtr,
    _pin: PhantomPinned,
}

impl RawNode {
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
