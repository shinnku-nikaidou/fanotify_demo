use nix::sys::fanotify::{EventFFlags, Fanotify, InitFlags, MarkFlags, MaskFlags};
use std::{fs, os::unix::io::AsRawFd};
use std::os::unix::fs::PermissionsExt;

fn check_kernel_version() {
    println!("DEBUG: Checking kernel version and fanotify support...");
    match std::fs::read_to_string("/proc/version") {
        Ok(version) => {
            println!("DEBUG: Kernel version: {}", version.trim());
        }
        Err(e) => {
            println!("DEBUG: Failed to read kernel version: {}", e);
        }
    }
    
    // Check if fanotify is available in kernel
    match std::fs::metadata("/proc/sys/fs/fanotify") {
        Ok(_) => println!("DEBUG: ✓ fanotify support detected in kernel"),
        Err(_) => println!("DEBUG: ⚠ fanotify support not detected or not accessible"),
    }
}

fn check_capabilities() {
    println!("DEBUG: Checking process capabilities...");
    match std::fs::read_to_string("/proc/self/status") {
        Ok(status) => {
            for line in status.lines() {
                if line.starts_with("Cap") {
                    println!("DEBUG: {}", line);
                }
            }
        }
        Err(e) => {
            println!("DEBUG: Failed to read process status: {}", e);
        }
    }
}

fn main() -> nix::Result<()> {
    println!("=== Starting fanotify filesystem monitoring program ===");
    
    // Check kernel and system support
    check_kernel_version();
    check_capabilities();
    
    // Check if running as root - use libc directly for compatibility
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    println!("DEBUG: Running as UID: {}, GID: {}", uid, gid);
    
    if uid != 0 {
        eprintln!("WARNING: fanotify typically requires root privileges");
        eprintln!("If you encounter permission errors, try running with sudo");
        eprintln!("Some fanotify features (like FAN_ATTRIB) require CAP_SYS_ADMIN capability");
    }
    
    // Initialize fanotify with basic settings
    println!("DEBUG: Initializing fanotify with FAN_CLASS_NOTIF and O_RDONLY...");
    println!("DEBUG: InitFlags::FAN_CLASS_NOTIF = {:?}", InitFlags::FAN_CLASS_NOTIF);
    println!("DEBUG: EventFFlags::O_RDONLY = {:?}", EventFFlags::O_RDONLY);
    
    let fan = match Fanotify::init(
        InitFlags::FAN_CLASS_NOTIF,
        EventFFlags::O_RDONLY,
    ) {
        Ok(f) => {
            println!("✓ Successfully initialized fanotify");
            println!("DEBUG: fanotify file descriptor created successfully");
            f
        },
        Err(e) => {
            eprintln!("✗ Failed to initialize fanotify: {}", e);
            eprintln!("DEBUG: errno details: {:?}", e);
            match e {
                nix::errno::Errno::EPERM => {
                    eprintln!("EPERM: Operation not permitted - need root privileges or CAP_SYS_ADMIN");
                }
                nix::errno::Errno::ENOSYS => {
                    eprintln!("ENOSYS: Function not implemented - fanotify not supported by kernel");
                }
                nix::errno::Errno::EINVAL => {
                    eprintln!("EINVAL: Invalid argument - check fanotify flags");
                }
                _ => {
                    eprintln!("Other error occurred during fanotify initialization");
                }
            }
            eprintln!("This usually happens due to insufficient permissions or missing kernel support");
            return Err(e);
        }
    };

    // Monitor file attribute and metadata change events - trying to add ATTRIB
    println!("DEBUG: Setting up monitoring mask with metadata change events...");
    
    // First try with FAN_ATTRIB
    let mask_with_attrib = MaskFlags::FAN_OPEN | MaskFlags::FAN_MODIFY | MaskFlags::FAN_CLOSE_WRITE | MaskFlags::FAN_ATTRIB;
    let mask_basic = MaskFlags::FAN_OPEN | MaskFlags::FAN_MODIFY | MaskFlags::FAN_CLOSE_WRITE;
    
    println!("DEBUG: mask_with_attrib = {:?}", mask_with_attrib);
    println!("DEBUG: mask_basic = {:?}", mask_basic);
    println!("DEBUG: Attempting to use ATTRIB flag for metadata monitoring...");
    println!("DEBUG: FAN_ATTRIB monitors: chmod, chown, utime, truncate, link/unlink");

    // Create a test file to monitor
    println!("DEBUG: Creating test file for monitoring...");
    let test_file_path = "/tmp/fanotify_test_file.txt";
    
    // Remove existing file first
    let _ = std::fs::remove_file(test_file_path);
    
    match std::fs::write(test_file_path, "initial content\n") {
        Ok(_) => {
            println!("✓ Created test file: {}", test_file_path);
            // Get file metadata
            match std::fs::metadata(test_file_path) {
                Ok(metadata) => {
                    println!("DEBUG: Initial file size: {} bytes", metadata.len());
                    println!("DEBUG: Initial file permissions: {:o}", metadata.permissions().mode());
                }
                Err(e) => println!("DEBUG: Failed to get file metadata: {}", e),
            }
        },
        Err(e) => {
            eprintln!("✗ Failed to create test file: {}", e);
            return Err(nix::errno::Errno::EIO);
        }
    }

    // Monitor the specific test file
    println!("DEBUG: Opening test file for monitoring...");
    let test_file = match std::fs::File::open(test_file_path) {
        Ok(file) => {
            println!("✓ Successfully opened test file");
            file
        },
        Err(e) => {
            eprintln!("✗ Failed to open test file: {}", e);
            return Err(nix::errno::Errno::ENOENT);
        }
    };
    
    println!("DEBUG: Adding fanotify mark to test file...");
    println!("DEBUG: Using MarkFlags::FAN_MARK_ADD = {:?}", MarkFlags::FAN_MARK_ADD);
    
    let mask = match fan.mark(
        MarkFlags::FAN_MARK_ADD,
        mask_with_attrib,
        &test_file,
        None::<&std::path::Path>,
    ) {
        Ok(_) => {
            println!("✓ Successfully marked test file for monitoring (with ATTRIB)");
            println!("DEBUG: FAN_ATTRIB support confirmed on this system");
            mask_with_attrib
        },
        Err(e) => {
            println!("⚠ Failed to mark with ATTRIB flag: {}", e);
            println!("DEBUG: errno details: {:?}", e);
            match e {
                nix::errno::Errno::EINVAL => {
                    println!("DEBUG: EINVAL - FAN_ATTRIB may not be supported on this kernel version");
                    println!("DEBUG: FAN_ATTRIB requires Linux kernel 5.1+ for file events");
                }
                nix::errno::Errno::EPERM => {
                    println!("DEBUG: EPERM - Need additional privileges for FAN_ATTRIB");
                }
                _ => {
                    println!("DEBUG: Other error with FAN_ATTRIB: {}", e);
                }
            }
            println!("DEBUG: Retrying with basic flags only...");
            match fan.mark(
                MarkFlags::FAN_MARK_ADD,
                mask_basic,
                &test_file,
                None::<&std::path::Path>,
            ) {
                Ok(_) => {
                    println!("✓ Successfully marked test file for monitoring (basic flags)");
                    println!("DEBUG: Basic fanotify events (OPEN, MODIFY, CLOSE_WRITE) will work");
                    mask_basic
                },
                Err(e2) => {
                    eprintln!("✗ Failed to mark test file even with basic flags: {}", e2);
                    eprintln!("DEBUG: errno details: {:?}", e2);
                    eprintln!("DEBUG: This indicates a fundamental fanotify issue");
                    return Err(e2);
                }
            }
        }
    };

    println!("=== Starting to listen for filesystem events ===");
    if mask.contains(MaskFlags::FAN_ATTRIB) {
        println!("Event types being monitored: OPEN, MODIFY, CLOSE_WRITE, ATTRIB (metadata)");
        println!("✓ FAN_ATTRIB is active - metadata changes will be detected");
    } else {
        println!("Event types being monitored: OPEN, MODIFY, CLOSE_WRITE (ATTRIB not supported)");
        println!("⚠ FAN_ATTRIB is not active - only content changes will be detected");
    }
    println!("Monitoring file: {}", test_file_path);
    println!("Press Ctrl+C to exit the program");
    println!("\n=== Test Commands (run in another terminal) ===");
    println!("Content modification tests:");
    println!("  echo 'new content' >> {}", test_file_path);
    println!("  cat {} > /dev/null", test_file_path);
    println!("Metadata modification tests (should trigger FAN_ATTRIB if supported):");
    println!("  chmod 755 {}", test_file_path);
    println!("  chmod 644 {}", test_file_path);
    println!("  touch {} # update timestamp", test_file_path);
    println!("  chown $USER:$USER {} # change ownership", test_file_path);
    println!("=== End Test Commands ===\n");
    println!("Waiting for events...\n");

    let mut event_count = 0;
    println!("DEBUG: Entering event loop, waiting for fanotify events...");
    
    loop {
        println!("DEBUG: Calling fan.read_events()...");
        match fan.read_events() {
            Ok(events) => {
                println!("DEBUG: Received {} events", events.len());
                if events.is_empty() {
                    println!("DEBUG: No events received, continuing...");
                    continue;
                }
                
                for ev in events {
                    event_count += 1;
                    let mask = ev.mask();
                    let pid = ev.pid();
                    
                    println!("\n=== EVENT #{} ===", event_count);
                    println!("DEBUG: Raw event mask: {:?}", mask);
                    println!("DEBUG: Event PID: {}", pid);
                    println!("DEBUG: Event version: {:?}", ev.version());
                    
                    // Decode individual mask flags
                    println!("DEBUG: Mask flag analysis:");
                    println!("  FAN_OPEN: {}", mask.contains(MaskFlags::FAN_OPEN));
                    println!("  FAN_MODIFY: {}", mask.contains(MaskFlags::FAN_MODIFY));
                    println!("  FAN_CLOSE_WRITE: {}", mask.contains(MaskFlags::FAN_CLOSE_WRITE));
                    println!("  FAN_ATTRIB: {}", mask.contains(MaskFlags::FAN_ATTRIB));
                    
                    // Get file path and close file descriptor properly
                    let path_info = if let Some(fd) = ev.fd() {
                        let fd_raw = fd.as_raw_fd();
                        println!("DEBUG: Event has file descriptor: {}", fd_raw);
                        let link = format!("/proc/self/fd/{}", fd_raw);
                        println!("DEBUG: Attempting to resolve path via: {}", link);
                        let result = match fs::read_link(&link) {
                            Ok(path) => {
                                let path_str = path.display().to_string();
                                println!("DEBUG: ✓ Resolved path: {}", path_str);
                                format!("path={}", path_str)
                            },
                            Err(e) => {
                                println!("DEBUG: ✗ Failed to resolve path for fd {}: {}", fd_raw, e);
                                // Try alternative method
                                match std::fs::read_to_string(format!("/proc/self/fdinfo/{}", fd_raw)) {
                                    Ok(fdinfo) => {
                                        println!("DEBUG: fdinfo content: {}", fdinfo.lines().take(3).collect::<Vec<_>>().join("; "));
                                    }
                                    Err(_) => {
                                        println!("DEBUG: Could not read fdinfo either");
                                    }
                                }
                                format!("fd={}", fd_raw)
                            },
                        };
                        // Don't manually close the fd - let the event handle it
                        // The fd will be automatically closed when the event is dropped
                        println!("DEBUG: Event file descriptor will be auto-closed");
                        result
                    } else {
                        println!("DEBUG: ⚠ No file descriptor in event (this is unusual)");
                        "path=unknown".to_string()
                    };

                    // Print different information based on event type
                    println!("\n📋 EVENT SUMMARY:");
                    let mut event_types = Vec::new();
                    
                    if mask.contains(MaskFlags::FAN_OPEN) {
                        println!("🔓 [OPEN] pid={} {} - File opened for reading/writing", pid, path_info);
                        event_types.push("OPEN");
                    }
                    if mask.contains(MaskFlags::FAN_MODIFY) {
                        println!("📝 [MODIFY] pid={} {} - File content was modified", pid, path_info);
                        event_types.push("MODIFY");
                    }
                    if mask.contains(MaskFlags::FAN_CLOSE_WRITE) {
                        println!("💾 [CLOSE_WRITE] pid={} {} - Writable file was closed", pid, path_info);
                        event_types.push("CLOSE_WRITE");
                    }
                    if mask.contains(MaskFlags::FAN_ATTRIB) {
                        println!("🔧 [ATTRIB] pid={} {} - File metadata/attributes changed", pid, path_info);
                        println!("   └─ This could be: chmod (permissions), chown (ownership), utime (timestamps), truncate, etc.");
                        event_types.push("ATTRIB");
                    }
                    
                    if event_types.is_empty() {
                        println!("❓ [UNKNOWN] pid={} {} - Unrecognized event type", pid, path_info);
                    }
                    
                    println!("📊 Event types detected: {}", event_types.join(", "));
                    
                    // Try to get current file status for comparison
                    if let Ok(path_str) = path_info.strip_prefix("path=").ok_or("") {
                        match std::fs::metadata(path_str) {
                            Ok(metadata) => {
                                println!("📁 Current file status:");
                                println!("   Size: {} bytes", metadata.len());
                                println!("   Permissions: {:o}", metadata.permissions().mode());
                                println!("   Modified: {:?}", metadata.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH));
                            }
                            Err(e) => {
                                println!("📁 Could not get current file status: {}", e);
                            }
                        }
                    }
                    
                    println!("==========================================");
                }
            },
            Err(e) => {
                eprintln!("\n✗ Error reading fanotify events: {}", e);
                eprintln!("DEBUG: errno details: {:?}", e);
                match e {
                    nix::errno::Errno::EINTR => {
                        eprintln!("DEBUG: EINTR - Interrupted system call, this is normal");
                        continue;
                    }
                    nix::errno::Errno::EAGAIN => {
                        eprintln!("DEBUG: EAGAIN - No events available right now");
                        continue;
                    }
                    _ => {
                        eprintln!("DEBUG: Serious error occurred: {}", e);
                        eprintln!("This might indicate a system error or interrupted operation");
                        return Err(e);
                    }
                }
            }
        }
    }
}
