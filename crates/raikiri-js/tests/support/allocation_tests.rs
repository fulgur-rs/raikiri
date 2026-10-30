#![allow(unsafe_code)]
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

#[derive(Clone, Copy, Default, Debug)]
pub struct Allocation {
    pub bytes: usize,
    pub calls: usize,
}
thread_local! {
    static CURRENT: Cell<Option<Allocation>> = const { Cell::new(None) };
}
fn record(bytes: usize) {
    let _ = CURRENT.try_with(|current| {
        if let Some(mut value) = current.get() {
            value.bytes = value.bytes.saturating_add(bytes);
            value.calls = value.calls.saturating_add(1);
            current.set(Some(value));
        }
    });
}
pub struct CountingAllocator;
// SAFETY: Every operation forwards the original pointer and layout to System.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller supplies a valid allocation layout.
        let result = unsafe { System.alloc(layout) };
        if !result.is_null() {
            record(layout.size());
        }
        result
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller supplies a valid allocation layout.
        let result = unsafe { System.alloc_zeroed(layout) };
        if !result.is_null() {
            record(layout.size());
        }
        result
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: The caller supplies the live pointer, its layout, and valid size.
        let result = unsafe { System.realloc(ptr, layout, size) };
        if !result.is_null() {
            record(size);
        }
        result
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: The caller supplies the live pointer and its allocation layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}
pub fn allocated_by<T>(run: impl FnOnce() -> T) -> (T, Allocation) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            CURRENT.set(None);
        }
    }
    assert!(
        CURRENT.get().is_none(),
        "allocation measurements cannot nest"
    );
    CURRENT.set(Some(Allocation::default()));
    let reset = Reset;
    let result = run();
    let allocation = CURRENT.get().unwrap();
    drop(reset);
    (result, allocation)
}

use raikiri_dom::Document;
use raikiri_js::runtime::{BoxGeometry, DocumentHost, DomRuntime, HostError};
use std::any::Any;

pub struct Host(pub Document);
impl DocumentHost for Host {
    fn document(&self) -> &Document {
        &self.0
    }
    fn document_mut(&mut self) -> &mut Document {
        &mut self.0
    }
    fn flush(&mut self) -> Result<(), HostError> {
        panic!("unexpected layout")
    }
    fn box_geometry(&mut self, _: usize) -> Result<Option<BoxGeometry>, HostError> {
        panic!("unexpected geometry")
    }
    fn computed_value(&mut self, _: usize, _: &str) -> Result<Option<String>, HostError> {
        panic!("unexpected style")
    }
    fn parse_fragment(&mut self, _: &str, _: &str, _: &str) -> Result<Document, HostError> {
        panic!("unexpected parser")
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

pub fn runtime() -> DomRuntime {
    DomRuntime::new(Host(Document::new())).unwrap()
}
