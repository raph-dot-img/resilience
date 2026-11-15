//! The viewer's own memory limit (ADR 0016 §7): whatever a hostile file
//! makes SQLite or the decoders allocate, the process cannot take more
//! than this much address space, so a bug ends the viewer, never the
//! machine.

use std::io;

/// RLIMIT_AS for the viewer and `info`: 1 GiB.
pub const ADDRESS_SPACE_LIMIT: u64 = 1 << 30;

/// Lower the soft and hard RLIMIT_AS of this process to at most `bytes`.
/// A limit that is already lower is kept. The hard limit is lowered too, so
/// nothing later in the process can raise it again.
pub fn limit_address_space(bytes: u64) -> io::Result<()> {
    let mut cur = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit writes one rlimit into the valid, aligned struct.
    if unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut cur) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // RLIM_INFINITY is the largest value, so `min` keeps any lower limit.
    let hard = cur.rlim_max.min(bytes);
    let new = libc::rlimit {
        rlim_cur: cur.rlim_cur.min(hard),
        rlim_max: hard,
    };
    // SAFETY: setrlimit reads one valid rlimit; lowering needs no privilege.
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &new) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The current soft RLIMIT_AS (for tests and diagnostics).
pub fn address_space_limit() -> io::Result<u64> {
    let mut cur = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: as in `limit_address_space`.
    if unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut cur) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(cur.rlim_cur)
}
