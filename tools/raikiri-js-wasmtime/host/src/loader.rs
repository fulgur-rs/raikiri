use std::ptr::NonNull;
use wasmtime::{CustomCodeMemory, Engine, Module};

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
compile_error!("zero-copy spike supports Linux x86_64 only");

#[repr(C, align(4096))]
struct Aligned<const N: usize>([u8; N]);
static AOT: Aligned<{ include_bytes!(env!("RAIKIRI_JS_AOT")).len() }> =
    Aligned(*include_bytes!(env!("RAIKIRI_JS_AOT")));

pub(crate) fn artifact() -> &'static [u8] {
    &AOT.0
}

pub(crate) fn load(engine: &Engine) -> anyhow::Result<Module> {
    let memory = NonNull::from(artifact());
    // SAFETY: exact trusted compiler output is page-aligned static storage,
    // remains alive and byte-immutable for every Module lifetime. Only its
    // isolated text pages change permissions via the publisher below.
    Ok(unsafe { Module::deserialize_raw(engine, memory)? })
}

pub(crate) struct Publisher;
impl Publisher {
    fn protect(
        ptr: *const u8,
        len: usize,
        flags: rustix::mm::MprotectFlags,
    ) -> wasmtime::Result<()> {
        let base = artifact().as_ptr() as usize;
        let address = ptr as usize;
        wasmtime::ensure!(
            address.is_multiple_of(4096) && len.is_multiple_of(4096),
            "unaligned code range"
        );
        wasmtime::ensure!(
            address >= base
                && address
                    .checked_add(len)
                    .is_some_and(|end| end <= base + artifact().len()),
            "code range outside embedded artifact"
        );
        if len != 0 {
            // SAFETY: page-aligned in-bounds artifact pages remain mapped forever.
            // Only READ or READ|EXEC protections are granted; bytes remain immutable.
            unsafe {
                rustix::mm::mprotect(ptr.cast_mut().cast(), len, flags)?;
            }
        }
        Ok(())
    }
}
impl CustomCodeMemory for Publisher {
    fn required_alignment(&self) -> usize {
        4096
    }
    fn publish_executable(&self, ptr: *const u8, len: usize) -> wasmtime::Result<()> {
        Self::protect(
            ptr,
            len,
            rustix::mm::MprotectFlags::READ | rustix::mm::MprotectFlags::EXEC,
        )
    }
    fn unpublish_executable(&self, ptr: *const u8, len: usize) -> wasmtime::Result<()> {
        // Unlike writable heap code, immutable embedded code returns to READ.
        // Wasmtime guarantees no executing code remains when this is called.
        Self::protect(ptr, len, rustix::mm::MprotectFlags::READ)
    }
}
