// SPDX-License-Identifier: MPL-2.0
//
// list.rs -- instrusive doubly linked list
// Copyright (C) 2026  Jacob Koziej <jacobkoziej@gmail.com>

#![feature(allocator_api)]
#![feature(const_atomic)]
#![allow(dead_code)]

use core::alloc::Allocator;
use core::cell::UnsafeCell;
use core::iter::{
    DoubleEndedIterator, ExactSizeIterator, FromIterator, FusedIterator, IntoIterator, Iterator,
};
use core::marker::{PhantomData, PhantomPinned};
use core::mem::{ManuallyDrop, offset_of};
use core::ops::{Deref, Drop};
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};
use std::alloc::{AllocatorClone, Global};
use std::sync::Arc;

struct RawNode {
    prev: AtomicPtr<Self>,
    next: AtomicPtr<Self>,
    _pin: PhantomPinned,
}

impl RawNode {
    const fn insert(prev: *const Self, node: *const Self, next: *const Self) {
        unsafe {
            (*prev).next.store(node.cast_mut(), Ordering::Relaxed);
            (*node).prev.store(prev.cast_mut(), Ordering::Relaxed);
            (*node).next.store(next.cast_mut(), Ordering::Relaxed);
            (*next).prev.store(node.cast_mut(), Ordering::Relaxed);
        }
    }

    fn is_singleton(&self) -> bool {
        let ptr = ptr::from_ref(self).cast_mut();
        let prev = self.prev.load(Ordering::Relaxed);
        let next = self.next.load(Ordering::Relaxed);

        prev == ptr && next == ptr
    }

    const fn new() -> Self {
        Self {
            prev: AtomicPtr::new(ptr::null_mut()),
            next: AtomicPtr::new(ptr::null_mut()),
            _pin: PhantomPinned,
        }
    }

    const fn next(ptr: *const Self) -> *const Self {
        unsafe { (*ptr).next.load(Ordering::Relaxed).cast_const() }
    }

    const fn prev(ptr: *const Self) -> *const Self {
        unsafe { (*ptr).prev.load(Ordering::Relaxed).cast_const() }
    }

    const fn release(ptr: *const Self) {
        unsafe {
            (*ptr).prev.store(ptr::null_mut(), Ordering::Relaxed);
            (*ptr).next.store(ptr::null_mut(), Ordering::Release);
        }
    }

    const fn remove(prev: *const Self, node: *const Self, next: *const Self) {
        unsafe {
            (*prev).next.store(next.cast_mut(), Ordering::Relaxed);
            (*next).prev.store(prev.cast_mut(), Ordering::Relaxed);

            (*node).prev.store(node.cast_mut(), Ordering::Relaxed);
            (*node).next.store(node.cast_mut(), Ordering::Relaxed);
        }
    }

    fn try_claim(ptr: *const Self) -> bool {
        let node = unsafe { &*ptr };

        let claimed = node
            .next
            .compare_exchange(
                ptr::null_mut(),
                ptr.cast_mut(),
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok();

        if !claimed {
            return false;
        }

        node.prev.store(ptr.cast_mut(), Ordering::Relaxed);

        true
    }
}

impl IntoIterator for &RawNode {
    type Item = *const RawNode;
    type IntoIter = RawIter;

    fn into_iter(self) -> Self::IntoIter {
        Self::IntoIter::new(ptr::from_ref(self))
    }
}

struct RawIter {
    front: *const RawNode,
    back: *const RawNode,
}

impl RawIter {
    const fn new(node: *const RawNode) -> Self {
        if node.is_null() {
            return Self {
                front: ptr::null(),
                back: ptr::null(),
            };
        }

        Self {
            front: node,
            back: RawNode::prev(node),
        }
    }
}

impl DoubleEndedIterator for RawIter {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.back.is_null() {
            return None;
        }

        let node = self.back;

        if self.front == self.back {
            self.front = ptr::null();
            self.back = ptr::null();
        } else {
            self.back = RawNode::prev(self.back);
        }

        Some(node)
    }
}

impl FusedIterator for RawIter {}

impl Iterator for RawIter {
    type Item = *const RawNode;

    fn next(&mut self) -> Option<Self::Item> {
        if self.front.is_null() {
            return None;
        }

        let node = self.front;

        if self.front == self.back {
            self.front = ptr::null();
            self.back = ptr::null();
        } else {
            self.front = RawNode::next(self.front);
        }

        Some(node)
    }
}

pub trait Role {}

pub struct Node<T, R: Role> {
    raw: RawNode,
    _marker: PhantomData<fn() -> (T, R)>,
}

impl<T, R> Node<T, R>
where
    R: Role,
{
    const fn as_raw(ptr: *const Self) -> *const RawNode {
        unsafe { &raw const (*ptr).raw }
    }

    const unsafe fn from_raw(ptr: *const RawNode) -> *const Self {
        let offset = offset_of!(Self, raw);

        unsafe { ptr.byte_sub(offset).cast::<Self>() }
    }

    pub unsafe fn new() -> Self {
        Self {
            raw: RawNode::new(),
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
    unsafe fn as_item(node: *const Node<Self, R>) -> *const Self;
    fn as_node(ptr: *const Self) -> *const Node<Self, R>;
}

#[macro_export]
macro_rules! linked {
    ($ty:ty, $role:ty, $field:ident) => {
        unsafe impl $crate::Linked<$role> for $ty {
            unsafe fn as_item(node: *const $crate::Node<Self, $role>) -> *const Self {
                let offset = ::core::mem::offset_of!(Self, $field);

                unsafe { node.byte_sub(offset).cast::<Self>() }
            }

            fn as_node(ptr: *const Self) -> *const $crate::Node<Self, $role> {
                unsafe { &raw const (*ptr).$field }
            }
        }
    };
}

pub struct ListArc<T, R, A = Global>
where
    T: Linked<R>,
    R: Role,
    A: Allocator,
{
    arc: Arc<T, A>,
    _marker: PhantomData<fn() -> R>,
}

impl<T, R, A> ListArc<T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: Allocator,
{
    unsafe fn from_raw_in(ptr: *const T, alloc: A) -> Self {
        Self {
            arc: unsafe { Arc::from_raw_in(ptr, alloc) },
            _marker: PhantomData,
        }
    }

    fn into_raw(self) -> *const T {
        let this = ManuallyDrop::new(self);
        let arc = unsafe { ptr::read(&this.arc) };
        let (ptr, _alloc) = Arc::into_raw_with_allocator(arc);

        ptr
    }
}

impl<T, R, A> ListArc<T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
    pub fn try_from_arc(arc: &Arc<T, A>) -> Option<Self> {
        let node = T::as_node(&**arc);

        if !RawNode::try_claim(Node::<T, R>::as_raw(node)) {
            return None;
        }

        Some(Self {
            arc: Arc::clone(arc),
            _marker: PhantomData,
        })
    }
}

impl<T, R, A> Deref for ListArc<T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: Allocator,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.arc
    }
}

impl<T, R, A> Drop for ListArc<T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: Allocator,
{
    fn drop(&mut self) {
        let node = T::as_node(&*self.arc);

        RawNode::release(Node::<T, R>::as_raw(node));
    }
}

pub struct List<T, R, A = Global>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
    ptr: *const RawNode,
    len: usize,
    alloc: A,
    _marker: PhantomData<fn() -> (T, R)>,
}

impl<T, R> List<T, R>
where
    T: Linked<R>,
    R: Role,
{
    pub const fn new() -> Self {
        Self::new_in(Global)
    }
}

impl<T, R, A> List<T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
    fn get_item(&self, ptr: *const RawNode) -> Arc<T, A> {
        let node = unsafe { Node::<T, R>::from_raw(ptr) };
        let item = unsafe { T::as_item(node) };

        let arc = ManuallyDrop::new(unsafe { Arc::from_raw_in(item, self.alloc.clone()) });

        Arc::clone(&arc)
    }

    pub fn head(&self) -> Option<Arc<T, A>> {
        if self.is_empty() {
            return None;
        }

        let head = self.ptr;

        Some(self.get_item(head))
    }

    pub const fn is_empty(&self) -> bool {
        self.ptr.is_null()
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn new_in(alloc: A) -> Self {
        Self {
            ptr: ptr::null(),
            len: 0,
            alloc,
            _marker: PhantomData,
        }
    }

    pub fn pop_back(&mut self) -> Option<ListArc<T, R, A>> {
        if self.is_empty() {
            return None;
        }

        let head = self.ptr;
        let tail = RawNode::prev(head);

        if unsafe { &*head }.is_singleton() {
            self.ptr = ptr::null();
        }

        RawNode::remove(RawNode::prev(tail), tail, head);

        self.len -= 1;

        let node = unsafe { Node::<T, R>::from_raw(tail) };
        let item = unsafe { T::as_item(node) };

        Some(unsafe { ListArc::from_raw_in(item, self.alloc.clone()) })
    }

    pub fn pop_front(&mut self) -> Option<ListArc<T, R, A>> {
        if self.is_empty() {
            return None;
        }

        let head = self.ptr;
        let tail = RawNode::prev(head);

        if unsafe { &*head }.is_singleton() {
            self.ptr = ptr::null();
        } else {
            self.ptr = RawNode::next(head);
        }

        RawNode::remove(tail, head, RawNode::next(head));

        self.len -= 1;

        let node = unsafe { Node::<T, R>::from_raw(head) };
        let item = unsafe { T::as_item(node) };

        Some(unsafe { ListArc::from_raw_in(item, self.alloc.clone()) })
    }

    pub fn push_front(&mut self, arc: ListArc<T, R, A>) {
        let node = Node::<T, R>::as_raw(T::as_node(arc.into_raw()));

        if self.is_empty() {
            RawNode::insert(node, node, node);
        } else {
            let head = self.ptr;
            let tail = RawNode::prev(head);

            RawNode::insert(tail, node, head);
        }

        self.ptr = node;
        self.len += 1;
    }

    pub fn push_back(&mut self, arc: ListArc<T, R, A>) {
        let node = Node::<T, R>::as_raw(T::as_node(arc.into_raw()));

        if self.is_empty() {
            RawNode::insert(node, node, node);

            self.ptr = node;
            self.len += 1;

            return;
        }

        let head = self.ptr;
        let tail = RawNode::prev(head);

        RawNode::insert(tail, node, head);

        self.len += 1;
    }

    pub fn tail(&self) -> Option<Arc<T, A>> {
        if self.is_empty() {
            return None;
        }

        let head = self.ptr;
        let tail = RawNode::prev(head);

        Some(self.get_item(tail))
    }
}

impl<T, R, A> Drop for List<T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
    fn drop(&mut self) {
        while self.pop_front().is_some() {}
    }
}

impl<T, R> FromIterator<ListArc<T, R>> for List<T, R>
where
    T: Linked<R>,
    R: Role,
{
    fn from_iter<I: IntoIterator<Item = ListArc<T, R>>>(iter: I) -> Self {
        let mut list = List::<T, R>::new();

        for i in iter {
            list.push_back(i)
        }

        list
    }
}

impl<T, R, A> IntoIterator for List<T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
    type Item = ListArc<T, R, A>;
    type IntoIter = IntoIter<T, R, A>;

    fn into_iter(self) -> Self::IntoIter {
        Self::IntoIter::new(self)
    }
}

impl<'a, T, R, A> IntoIterator for &'a List<T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
    type Item = &'a T;
    type IntoIter = Iter<'a, T, R, A>;

    fn into_iter(self) -> Self::IntoIter {
        Self::IntoIter::new(self)
    }
}

unsafe impl<T, R, A> Send for List<T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone + Send,
{
}

pub struct Iter<'a, T, R, A = Global>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
    list: &'a List<T, R, A>,
    len: usize,
    raw: RawIter,
}

impl<'a, T, R, A> Iter<'a, T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
    pub fn new(list: &'a List<T, R, A>) -> Self {
        Self {
            list: list,
            len: list.len(),
            raw: RawIter::new(list.ptr),
        }
    }
}

impl<'a, T, R, A> DoubleEndedIterator for Iter<'a, T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
    fn next_back(&mut self) -> Option<Self::Item> {
        let ptr = self.raw.next_back()?;

        self.len -= 1;

        let node = unsafe { Node::<T, R>::from_raw(ptr) };

        Some(unsafe { &*T::as_item(node) })
    }
}

impl<'a, T, R, A> ExactSizeIterator for Iter<'a, T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
    fn len(&self) -> usize {
        self.len
    }
}

impl<'a, T, R, A> FusedIterator for Iter<'a, T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
}

impl<'a, T, R, A> Iterator for Iter<'a, T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        let ptr = self.raw.next()?;

        self.len -= 1;

        let node = unsafe { Node::<T, R>::from_raw(ptr) };

        Some(unsafe { &*T::as_item(node) })
    }
}

pub struct IntoIter<T, R, A = Global>(List<T, R, A>)
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone;

impl<T, R, A> IntoIter<T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
    pub fn new(list: List<T, R, A>) -> Self {
        Self(list)
    }
}

impl<T, R, A> DoubleEndedIterator for IntoIter<T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.pop_back()
    }
}

impl<T, R, A> ExactSizeIterator for IntoIter<T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
    fn len(&self) -> usize {
        self.0.len()
    }
}

impl<T, R, A> FusedIterator for IntoIter<T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
}

impl<T, R, A> Iterator for IntoIter<T, R, A>
where
    T: Linked<R>,
    R: Role,
    A: AllocatorClone,
{
    type Item = ListArc<T, R, A>;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.pop_front()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use core::pin::{Pin, pin};

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

    const _: () = {
        fn check<T: Send + Sync>() {}
        let _ = check::<ListArc<Item, Foo>>;
    };

    const _: () = {
        fn check<T: Send>() {}
        let _ = check::<List<Item, Foo>>;
    };

    fn ptr(node: Pin<&RawNode>) -> *const RawNode {
        ptr::from_ref(node.get_ref())
    }

    fn token(data: u32) -> ListArc<Item, Foo> {
        ListArc::try_from_arc(&Arc::new(Item::new(data))).unwrap()
    }

    mod raw_node {
        use super::*;
        use core::pin::{Pin, pin};

        fn prev(node: Pin<&RawNode>) -> *const RawNode {
            RawNode::prev(ptr(node))
        }

        fn next(node: Pin<&RawNode>) -> *const RawNode {
            RawNode::next(ptr(node))
        }

        fn assert_ring(nodes: &[Pin<&RawNode>]) {
            let n = nodes.len();
            let expected: Vec<_> = nodes.iter().map(|node| ptr(*node)).collect();

            assert!(RawIter::new(expected[0]).eq(expected.iter().copied()));

            for i in 0..n {
                let prev = ptr(nodes[(i + n - 1) % n]);
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

            assert!(c.is_singleton());
            assert_ring(&[a, b]);

            RawNode::remove(ptr(b), ptr(a), ptr(b));

            assert!(a.is_singleton());
            assert!(b.is_singleton());

            RawNode::remove(ptr(b), ptr(b), ptr(b));

            assert!(b.is_singleton());
        }
    }

    mod raw_iter {
        use super::*;

        #[test]
        fn empty() {
            let mut iter = RawIter::new(ptr::null());

            assert_eq!(iter.next(), None);
            assert_eq!(iter.next_back(), None);
            assert_eq!(iter.next(), None);
            assert_eq!(iter.next_back(), None);
        }

        #[test]
        fn unlinked() {
            let node = pin!(RawNode::new());
            let node = node.into_ref();

            let mut iter = RawIter::new(ptr(node));

            assert_eq!(iter.next(), Some(ptr(node)));
            assert_eq!(iter.next(), None);
            assert_eq!(iter.next_back(), None);
        }

        #[test]
        fn singleton() {
            let node = pin!(RawNode::new());
            let node = node.into_ref();

            RawNode::insert(ptr(node), ptr(node), ptr(node));

            let mut iter = RawIter::new(ptr(node));

            assert_eq!(iter.next(), Some(ptr(node)));
            assert_eq!(iter.next(), None);
            assert_eq!(iter.next_back(), None);

            let mut iter = RawIter::new(ptr(node));

            assert_eq!(iter.next_back(), Some(ptr(node)));
            assert_eq!(iter.next_back(), None);
            assert_eq!(iter.next(), None);
        }

        #[test]
        fn forward_three() {
            let a = pin!(RawNode::new());
            let a = a.into_ref();
            let b = pin!(RawNode::new());
            let b = b.into_ref();
            let c = pin!(RawNode::new());
            let c = c.into_ref();

            RawNode::insert(ptr(b), ptr(a), ptr(b));
            RawNode::insert(ptr(b), ptr(c), ptr(a));

            assert!(RawIter::new(ptr(a)).eq([ptr(a), ptr(b), ptr(c)]));
        }

        #[test]
        fn into_iter_matches_new() {
            let a = pin!(RawNode::new());
            let a = a.into_ref();
            let b = pin!(RawNode::new());
            let b = b.into_ref();

            RawNode::insert(ptr(b), ptr(a), ptr(b));

            assert!(a.get_ref().into_iter().eq(RawIter::new(ptr(a))));
        }

        #[test]
        fn rev_three() {
            let a = pin!(RawNode::new());
            let a = a.into_ref();
            let b = pin!(RawNode::new());
            let b = b.into_ref();
            let c = pin!(RawNode::new());
            let c = c.into_ref();

            RawNode::insert(ptr(b), ptr(a), ptr(b));
            RawNode::insert(ptr(b), ptr(c), ptr(a));

            assert!(RawIter::new(ptr(a)).rev().eq([ptr(c), ptr(b), ptr(a)]));
        }

        #[test]
        fn both_ends_three() {
            let a = pin!(RawNode::new());
            let a = a.into_ref();
            let b = pin!(RawNode::new());
            let b = b.into_ref();
            let c = pin!(RawNode::new());
            let c = c.into_ref();

            RawNode::insert(ptr(b), ptr(a), ptr(b));
            RawNode::insert(ptr(b), ptr(c), ptr(a));

            let mut iter = RawIter::new(ptr(a));

            assert_eq!(iter.next(), Some(ptr(a)));
            assert_eq!(iter.next_back(), Some(ptr(c)));
            assert_eq!(iter.next(), Some(ptr(b)));
            assert_eq!(iter.next(), None);
            assert_eq!(iter.next_back(), None);
        }
    }

    mod list_arc {
        use super::*;
        use std::env;
        use std::str::FromStr;
        use std::thread;

        fn env_read<T: FromStr>(name: &str, default: T) -> T {
            env::var(name)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(default)
        }

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

            let foo = unsafe { ListArc::<_, Foo>::from_raw_in(ptr, Global) };

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

        #[test]
        fn multithreaded_minting() {
            let threads = env_read::<usize>("LISTARC_THREADS", 8).max(1);
            let iters = env_read::<usize>("LISTARC_ITERATIONS", 1_000);

            let arc = Arc::new(Item::new(0));

            thread::scope(|s| {
                for _ in 0..threads {
                    let arc = &arc;

                    s.spawn(move || {
                        for _ in 0..iters {
                            drop(ListArc::<Item, Foo>::try_from_arc(arc));
                        }
                    });
                }
            });

            assert_eq!(Arc::strong_count(&arc), 1);
            assert!(ListArc::<_, Foo>::try_from_arc(&arc).is_some());
        }
    }

    mod list {
        use super::*;

        #[test]
        fn empty() {
            let mut list = List::<Item, Foo>::new();

            assert!(list.is_empty());
            assert_eq!(list.len(), 0);
            assert!(list.head().is_none());
            assert!(list.tail().is_none());
            assert!(list.pop_front().is_none());
            assert!(list.pop_back().is_none());
        }

        #[test]
        fn push_pop_front() {
            let mut list = List::new();

            for data in [0, 1, 2] {
                list.push_front(token(data));
            }

            assert!(!list.is_empty());
            assert_eq!(list.len(), 3);

            assert_eq!(list.head().unwrap().data, 2);
            assert_eq!(list.tail().unwrap().data, 0);

            assert_eq!(list.pop_front().unwrap().data, 2);

            assert_eq!(list.head().unwrap().data, 1);
            assert_eq!(list.tail().unwrap().data, 0);
        }

        #[test]
        fn push_pop_back() {
            let mut list = List::new();

            for data in [0, 1, 2] {
                list.push_back(token(data));
            }

            assert!(!list.is_empty());
            assert_eq!(list.len(), 3);

            assert_eq!(list.head().unwrap().data, 0);
            assert_eq!(list.tail().unwrap().data, 2);

            assert_eq!(list.pop_back().unwrap().data, 2);

            assert_eq!(list.head().unwrap().data, 0);
            assert_eq!(list.tail().unwrap().data, 1);
        }
    }

    mod iter {
        use super::*;

        #[test]
        fn iter_round_trip() {
            let src = [0, 1, 2];

            let list: List<Item, Foo> = src.into_iter().map(token).collect();

            assert!((&list).into_iter().map(|item| item.data).eq(src));
            assert!(
                (&list)
                    .into_iter()
                    .rev()
                    .map(|item| item.data)
                    .eq(src.into_iter().rev())
            );
        }

        #[test]
        fn into_iter_round_trip() {
            let src = [0, 1, 2];

            let forward: List<Item, Foo> = src.into_iter().map(token).collect();
            let reverse: List<Item, Foo> = src.into_iter().map(token).collect();

            assert!(forward.into_iter().map(|item| item.data).eq(src));
            assert!(
                reverse
                    .into_iter()
                    .rev()
                    .map(|item| item.data)
                    .eq(src.into_iter().rev())
            );
        }

        #[test]
        fn iter_exact_size() {
            let list = List::<Item, Foo>::new();
            let mut iter = (&list).into_iter();

            assert_eq!(iter.len(), 0);
            assert!(iter.next().is_none());
            assert!(iter.next_back().is_none());
            assert_eq!(iter.len(), 0);

            let list: List<Item, Foo> = [0, 1, 2].into_iter().map(token).collect();
            let mut iter = (&list).into_iter();

            assert_eq!(iter.len(), 3);
            assert_eq!(iter.next().unwrap().data, 0);
            assert_eq!(iter.len(), 2);
            assert_eq!(iter.next_back().unwrap().data, 2);
            assert_eq!(iter.len(), 1);
            assert_eq!(iter.next().unwrap().data, 1);
            assert_eq!(iter.len(), 0);
            assert!(iter.next().is_none());
            assert!(iter.next_back().is_none());
            assert_eq!(iter.len(), 0);
        }

        #[test]
        fn into_iter_exact_size() {
            let list = List::<Item, Foo>::new();
            let mut iter = list.into_iter();

            assert_eq!(iter.len(), 0);
            assert!(iter.next().is_none());
            assert!(iter.next_back().is_none());
            assert_eq!(iter.len(), 0);

            let list: List<Item, Foo> = [0, 1, 2].into_iter().map(token).collect();
            let mut iter = list.into_iter();

            assert_eq!(iter.len(), 3);
            assert_eq!(iter.next().unwrap().data, 0);
            assert_eq!(iter.len(), 2);
            assert_eq!(iter.next_back().unwrap().data, 2);
            assert_eq!(iter.len(), 1);
            assert_eq!(iter.next().unwrap().data, 1);
            assert_eq!(iter.len(), 0);
            assert!(iter.next().is_none());
            assert!(iter.next_back().is_none());
            assert_eq!(iter.len(), 0);
        }
    }
}
