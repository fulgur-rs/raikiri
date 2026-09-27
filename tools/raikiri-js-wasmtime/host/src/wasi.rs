fn memory(caller: &mut wasmtime::Caller<'_, super::State>) -> wasmtime::Memory {
    caller.get_export("memory").unwrap().into_memory().unwrap()
}

// Minimal deterministic WASI services for this trusted reactor trial. No filesystem,
// network, or real randomness is granted. Unknown imports fail linking.
pub(crate) fn imports(linker: &mut wasmtime::Linker<super::State>) -> anyhow::Result<()> {
    let ns = "wasi_snapshot_preview1";

    linker.func_wrap(
        ns,
        "poll_oneoff",
        |_a: i32, _b: i32, _c: i32, _d: i32| -> wasmtime::Result<i32> {
            wasmtime::bail!("poll_oneoff unsupported in spike")
        },
    )?;
    linker.func_wrap(ns, "sched_yield", || -> i32 { 0 })?;
    linker.func_wrap(
        ns,
        "random_get",
        |mut caller: wasmtime::Caller<'_, super::State>, ptr: i32, len: i32| -> i32 {
            if len < 0 || len as usize > raikiri_js_wasmtime_protocol::MAX_MESSAGE {
                return 21;
            }
            let mem = memory(&mut caller);
            let Some(bytes) = mem
                .data_mut(&mut caller)
                .get_mut(ptr as usize..(ptr as usize).saturating_add(len as usize))
            else {
                return 21;
            };
            for (i, byte) in bytes.iter_mut().enumerate() {
                *byte = (i as u8).wrapping_mul(37).wrapping_add(11);
            }
            0
        },
    )?;
    linker.func_wrap(
        ns,
        "clock_time_get",
        |mut caller: wasmtime::Caller<'_, super::State>,
         _clock: i32,
         _precision: i64,
         ptr: i32|
         -> i32 {
            let mem = memory(&mut caller);
            if mem
                .write(&mut caller, ptr as usize, &1_000_000_000_u64.to_le_bytes())
                .is_err()
            {
                21
            } else {
                0
            }
        },
    )?;
    linker.func_wrap(
        ns,
        "environ_sizes_get",
        |mut caller: wasmtime::Caller<'_, super::State>, count: i32, size: i32| -> i32 {
            let mem = memory(&mut caller);
            if mem.write(&mut caller, count as usize, &[0; 4]).is_err()
                || mem.write(&mut caller, size as usize, &[0; 4]).is_err()
            {
                21
            } else {
                0
            }
        },
    )?;
    linker.func_wrap(
        ns,
        "environ_get",
        |_caller: wasmtime::Caller<'_, super::State>, _ptr: i32, _buf: i32| -> i32 { 0 },
    )?;
    linker.func_wrap(
        ns,
        "fd_write",
        |mut caller: wasmtime::Caller<'_, super::State>,
         _fd: i32,
         ptr: i32,
         len: i32,
         out: i32|
         -> i32 {
            let mem = memory(&mut caller);
            if !(0..=1024).contains(&len) {
                return 21;
            }
            let mut written = 0_u32;
            for i in 0..len as usize {
                let mut entry = [0_u8; 8];
                if mem
                    .read(
                        &caller,
                        (ptr as usize).saturating_add(i.saturating_mul(8)),
                        &mut entry,
                    )
                    .is_err()
                {
                    return 21;
                }
                written =
                    written.saturating_add(u32::from_le_bytes(entry[4..8].try_into().unwrap()));
            }
            if mem
                .write(&mut caller, out as usize, &written.to_le_bytes())
                .is_err()
            {
                21
            } else {
                0
            }
        },
    )?;
    linker.func_wrap(ns, "fd_close", |_fd: i32| -> i32 { 8 })?;
    linker.func_wrap(
        ns,
        "fd_seek",
        |_fd: i32, _offset: i64, _whence: i32, _ptr: i32| -> i32 { 8 },
    )?;

    linker.func_wrap(ns, "proc_exit", |code: i32| -> wasmtime::Result<()> {
        wasmtime::bail!("guest proc_exit({code})")
    })?;
    Ok(())
}
