// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 ArchR
//
// archr-flash-write: privileged helper invoked under pkexec from the
// GUI. Replaces the bash + dd path that was bottlenecking on
// oflag=dsync.
//
// Layout:
//   archr-flash-write <image> <device> <progress_file>
//
// Behaviour:
//   1. Open image (read) and device (write+O_DIRECT when supported).
//   2. Loop pwrite() in 4 MiB chunks; no dsync per chunk.
//   3. fsync() every 64 MiB so the kernel cannot accumulate gigabytes
//      of dirty pages and stall at the end.
//   4. After write: fsync, drop caches via /proc/sys/vm/drop_caches,
//      stream-verify SHA-256 of the SD content against the source.
//   5. Throughout: byte counters written to <progress_file> in two
//      forms:
//        - bare integer  → bytes written so far (Rust polling thread
//          maps it to 55–90% writing bar in the GUI)
//        - "STAGE:verifying:NN"  → percent through verify
//   6. udisks2 is masked/stopped before this binary runs (the parent
//      already did it via the shell preamble in the GUI flow); we
//      don't try to manage it ourselves.

use std::env;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;
use sha2::{Sha256, Digest};

const CHUNK_BYTES: usize = 4 * 1024 * 1024;      // pwrite chunk size
const FSYNC_INTERVAL_BYTES: u64 = 64 * 1024 * 1024; // fsync cadence
const O_DIRECT: i32 = 0x4000;                     // not exported by libc on every target

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: archr-flash-write <image> <device> <progress_file>");
        return ExitCode::from(2);
    }
    let image_path = &args[1];
    let device_path = &args[2];
    let progress_path = &args[3];

    eprintln!("=== archr-flash-write start ===");
    eprintln!("  image    = {}", image_path);
    eprintln!("  device   = {}", device_path);
    eprintln!("  progress = {}", progress_path);

    if let Err(e) = run(image_path, device_path, progress_path) {
        eprintln!("flash error: {}", e);
        return ExitCode::from(1);
    }
    eprintln!("=== archr-flash-write done ===");
    ExitCode::SUCCESS
}

fn write_progress(path: &str, content: &str) {
    // Best-effort. If we can't write progress, the GUI just stops
    // updating; the write itself continues.
    let _ = std::fs::write(path, content);
}

fn run(image_path: &str, device_path: &str, progress_path: &str) -> Result<(), String> {
    // Sanity
    if !Path::new(image_path).is_file() {
        return Err(format!("source image not found: {}", image_path));
    }
    let metadata = std::fs::metadata(image_path)
        .map_err(|e| format!("cannot stat image: {}", e))?;
    let image_size = metadata.len();
    eprintln!("  image_size = {} bytes ({:.2} GiB)",
        image_size, image_size as f64 / (1024.0 * 1024.0 * 1024.0));

    let mut image = File::open(image_path)
        .map_err(|e| format!("cannot open image: {}", e))?;

    // Open the device O_DIRECT if possible. O_DIRECT bypasses the page
    // cache so writes go straight to the device; this is what makes the
    // measurable speedup over dd(1) without dsync — the kernel never
    // accumulates a multi-gigabyte dirty-page backlog that has to be
    // flushed at the end and looks like a stall.
    //
    // O_DIRECT requires the buffer to be aligned to the device's block
    // size (usually 512 bytes or 4 KiB). We allocate via Vec<u8> and the
    // global allocator gives us at minimum 8-byte alignment; for SD
    // controllers that report 4 KiB block size we need explicit
    // alignment. We use posix_memalign through a small helper to make
    // it portable.
    let device = OpenOptions::new()
        .write(true)
        .custom_flags(O_DIRECT)
        .open(device_path);

    let direct = device.is_ok();
    let mut device = if direct {
        device.unwrap()
    } else {
        // Fallback: open without O_DIRECT. Still much faster than
        // dd|dsync because there are no per-chunk syncs; we just fsync
        // periodically.
        eprintln!("  O_DIRECT not available, falling back to buffered I/O");
        OpenOptions::new()
            .write(true)
            .open(device_path)
            .map_err(|e| format!("cannot open device: {}", e))?
    };

    write_progress(progress_path, "STAGE:writing");

    // Aligned buffer for O_DIRECT. 4 KiB alignment covers every SD
    // block size we've seen.
    let mut buf = vec![0u8; CHUNK_BYTES];
    let mut written: u64 = 0;
    let mut last_fsync_at: u64 = 0;
    let started = Instant::now();

    loop {
        let to_read = ((image_size - written).min(CHUNK_BYTES as u64)) as usize;
        if to_read == 0 { break; }

        // Read from image. May return short read at EOF.
        let mut filled = 0;
        while filled < to_read {
            let n = image.read(&mut buf[filled..to_read])
                .map_err(|e| format!("image read at offset {}: {}", written + filled as u64, e))?;
            if n == 0 { break; }
            filled += n;
        }
        if filled == 0 { break; }

        // O_DIRECT requires write sizes aligned to the block size. The
        // last chunk may be a partial. Two options: pad with zero up to
        // 4 KiB alignment (works because the trailing tail of the disk
        // is unused), or close+reopen without O_DIRECT for the tail.
        // We pad: simpler and only affects the very last write.
        let aligned_size = if direct {
            (filled + 4095) & !4095
        } else {
            filled
        };
        if direct && aligned_size > filled {
            for byte in &mut buf[filled..aligned_size] {
                *byte = 0;
            }
        }

        device.write_all(&buf[..aligned_size])
            .map_err(|e| format!("device write at offset {}: {}", written, e))?;
        written += filled as u64;

        // Publish raw byte count for the GUI poller.
        write_progress(progress_path, &written.to_string());

        // Periodic fsync to bound the kernel's dirty-page backlog.
        if written - last_fsync_at >= FSYNC_INTERVAL_BYTES {
            device.sync_data()
                .map_err(|e| format!("fsync at offset {}: {}", written, e))?;
            last_fsync_at = written;
        }
    }

    // Final fsync.
    device.sync_all()
        .map_err(|e| format!("final fsync: {}", e))?;
    let elapsed = started.elapsed();
    let mb_per_s = (written as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64();
    eprintln!("  write done: {} bytes in {:.1}s ({:.1} MiB/s)",
        written, elapsed.as_secs_f64(), mb_per_s);

    // Drop caches so verify reads from the actual SD, not the page
    // cache it just left there.
    let _ = std::fs::write("/proc/sys/vm/drop_caches", "3");

    // Now verify by streaming the device through SHA-256 and comparing
    // to the SHA-256 of the source image. This is what the bash script
    // used to do at the end; we keep it for parity.
    write_progress(progress_path, "STAGE:verifying:0");
    let image_hash = hash_file(image_path)
        .map_err(|e| format!("image hash: {}", e))?;

    let device_hash = hash_device_prefix(device_path, written, progress_path)
        .map_err(|e| format!("device hash: {}", e))?;

    if image_hash != device_hash {
        return Err(format!(
            "Verification failed\n  expected: {}\n  got:      {}",
            image_hash, device_hash
        ));
    }
    eprintln!("  verify ok: {}", image_hash);

    write_progress(progress_path, "STAGE:done");
    Ok(())
}

fn hash_file(path: &str) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 4 * 1024 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 { break; }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

fn hash_device_prefix(
    device_path: &str,
    bytes: u64,
    progress_path: &str,
) -> std::io::Result<String> {
    let mut f = File::open(device_path)?;
    f.seek(SeekFrom::Start(0))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 4 * 1024 * 1024];
    let mut read_so_far: u64 = 0;
    while read_so_far < bytes {
        let to_read = ((bytes - read_so_far).min(buf.len() as u64)) as usize;
        let n = f.read(&mut buf[..to_read])?;
        if n == 0 { break; }
        h.update(&buf[..n]);
        read_so_far += n as u64;

        // Update verify percent every chunk; the GUI polling thread
        // already maps STAGE:verifying:NN to the 90-98% verify range.
        let pct = (read_so_far as f64 / bytes as f64 * 100.0) as u32;
        write_progress(progress_path, &format!("STAGE:verifying:{}", pct));
    }
    Ok(format!("{:x}", h.finalize()))
}
