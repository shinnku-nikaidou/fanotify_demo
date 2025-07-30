use nix::sys::fanotify::{EventFFlags, Fanotify, InitFlags, MarkFlags, MaskFlags};
use std::{fs, os::unix::io::AsRawFd};

fn main() -> nix::Result<()> {
    println!("=== Starting fanotify filesystem monitoring program ===");
    
    // Check if running as root - use libc directly for compatibility
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    println!("DEBUG: Running as UID: {}, GID: {}", uid, gid);
    
    if uid != 0 {
        eprintln!("WARNING: fanotify typically requires root privileges");
        eprintln!("If you encounter permission errors, try running with sudo");
    }
    
    // Initialize fanotify with basic settings
    println!("DEBUG: Initializing fanotify with FAN_CLASS_NOTIF and O_RDONLY...");
    let fan = match Fanotify::init(
        InitFlags::FAN_CLASS_NOTIF,
        EventFFlags::O_RDONLY,
    ) {
        Ok(f) => {
            println!("✓ Successfully initialized fanotify");
            f
        },
        Err(e) => {
            eprintln!("✗ Failed to initialize fanotify: {}", e);
            eprintln!("This usually happens due to insufficient permissions or missing kernel support");
            return Err(e);
        }
    };

    // Monitor file attribute and metadata change events - trying to add ATTRIB
    println!("DEBUG: Setting up monitoring mask with metadata change events...");
    
    // First try with FAN_ATTRIB
    let mask_with_attrib = MaskFlags::FAN_OPEN | MaskFlags::FAN_MODIFY | MaskFlags::FAN_CLOSE_WRITE | MaskFlags::FAN_ATTRIB;
    let mask_basic = MaskFlags::FAN_OPEN | MaskFlags::FAN_MODIFY | MaskFlags::FAN_CLOSE_WRITE;
    
    println!("DEBUG: Attempting to use ATTRIB flag for metadata monitoring...");

    // Create a test file to monitor
    println!("DEBUG: Creating test file for monitoring...");
    let test_file_path = "/tmp/fanotify_test_file.txt";
    match std::fs::write(test_file_path, "initial content") {
        Ok(_) => println!("✓ Created test file: {}", test_file_path),
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
    let mask = match fan.mark(
        MarkFlags::FAN_MARK_ADD,
        mask_with_attrib,
        &test_file,
        None::<&std::path::Path>,
    ) {
        Ok(_) => {
            println!("✓ Successfully marked test file for monitoring (with ATTRIB)");
            mask_with_attrib
        },
        Err(e) => {
            println!("⚠ Failed to mark with ATTRIB flag: {}", e);
            println!("DEBUG: Retrying with basic flags only...");
            match fan.mark(
                MarkFlags::FAN_MARK_ADD,
                mask_basic,
                &test_file,
                None::<&std::path::Path>,
            ) {
                Ok(_) => {
                    println!("✓ Successfully marked test file for monitoring (basic flags)");
                    mask_basic
                },
                Err(e2) => {
                    eprintln!("✗ Failed to mark test file even with basic flags: {}", e2);
                    return Err(e2);
                }
            }
        }
    };

    println!("=== Starting to listen for filesystem events ===");
    if mask.contains(MaskFlags::FAN_ATTRIB) {
        println!("Event types being monitored: OPEN, MODIFY, CLOSE_WRITE, ATTRIB (metadata)");
    } else {
        println!("Event types being monitored: OPEN, MODIFY, CLOSE_WRITE (ATTRIB not supported)");
    }
    println!("Monitoring file: {}", test_file_path);
    println!("Press Ctrl+C to exit the program");
    println!("Try: echo 'new content' >> /tmp/fanotify_test_file.txt");
    println!("Or:   cat /tmp/fanotify_test_file.txt");
    println!("Or:   sudo chmod 755 /tmp/fanotify_test_file.txt");
    println!("Waiting for events...\n");

    let mut event_count = 0;
    loop {
        match fan.read_events() {
            Ok(events) => {
                for ev in events {
                    event_count += 1;
                    let mask = ev.mask();
                    let pid = ev.pid();
                    
                    println!("DEBUG: Event #{} received, mask: {:?}, pid: {}", event_count, mask, pid);
                    
                    // Get file path and close file descriptor properly
                    let path_info = if let Some(fd) = ev.fd() {
                        let fd_raw = fd.as_raw_fd();
                        let link = format!("/proc/self/fd/{}", fd_raw);
                        let result = match fs::read_link(&link) {
                            Ok(path) => {
                                let path_str = path.display().to_string();
                                println!("DEBUG: Resolved path: {}", path_str);
                                format!("path={}", path_str)
                            },
                            Err(e) => {
                                println!("DEBUG: Failed to resolve path for fd {}: {}", fd_raw, e);
                                format!("fd={}", fd_raw)
                            },
                        };
                        // Don't manually close the fd - let the event handle it
                        // The fd will be automatically closed when the event is dropped
                        println!("DEBUG: Event file descriptor will be auto-closed");
                        result
                    } else {
                        println!("DEBUG: No file descriptor in event");
                        "path=unknown".to_string()
                    };

                    // Print different information based on event type
                    if mask.contains(MaskFlags::FAN_OPEN) {
                        println!("🔓 [OPEN] pid={} {} - File opened", pid, path_info);
                    }
                    if mask.contains(MaskFlags::FAN_MODIFY) {
                        println!("📝 [MODIFY] pid={} {} - File content modified", pid, path_info);
                    }
                    if mask.contains(MaskFlags::FAN_CLOSE_WRITE) {
                        println!("💾 [CLOSE_WRITE] pid={} {} - Writable file closed", pid, path_info);
                    }
                    if mask.contains(MaskFlags::FAN_ATTRIB) {
                        println!("🔧 [ATTRIB] pid={} {} - File metadata/attributes changed", pid, path_info);
                    }
                    println!("---");
                }
            },
            Err(e) => {
                eprintln!("✗ Error reading fanotify events: {}", e);
                eprintln!("This might indicate a system error or interrupted operation");
                return Err(e);
            }
        }
    }
}
