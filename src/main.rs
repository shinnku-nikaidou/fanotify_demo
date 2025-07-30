use std::{fs, mem};
use std::os::unix::fs::PermissionsExt;

// fanotify constants and structures
const FAN_CLASS_NOTIF: u32 = 0;
const FAN_CLOEXEC: u32 = 0x00000001;

const FAN_OPEN: u64 = 0x00000001;
const FAN_CLOSE_WRITE: u64 = 0x00000008;
const FAN_MODIFY: u64 = 0x00000002;
const FAN_ATTRIB: u64 = 0x00000004;

const FAN_MARK_ADD: u32 = 0x00000001;
const FAN_MARK_ONLYDIR: u32 = 0x00000008;

const AT_FDCWD: libc::c_int = -100;

// fanotify_event_metadata structure
#[repr(C)]
#[derive(Debug)]
struct FanotifyEventMetadata {
    event_len: u32,
    vers: u8,
    reserved: u8,
    metadata_len: u16,
    mask: u64,
    fd: i32,
    pid: i32,
}

// System call numbers (x86_64)
const SYS_FANOTIFY_INIT: libc::c_long = 300;
const SYS_FANOTIFY_MARK: libc::c_long = 301;

// Raw system call wrappers
unsafe fn fanotify_init(flags: u32, event_f_flags: u32) -> libc::c_int {
    unsafe { libc::syscall(SYS_FANOTIFY_INIT, flags, event_f_flags) as libc::c_int }
}

unsafe fn fanotify_mark(
    fanotify_fd: libc::c_int,
    flags: u32,
    mask: u64,
    dirfd: libc::c_int,
    pathname: *const libc::c_char,
) -> libc::c_int {
    unsafe { libc::syscall(SYS_FANOTIFY_MARK, fanotify_fd, flags, mask, dirfd, pathname) as libc::c_int }
}

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

fn get_errno() -> i32 {
    unsafe { *libc::__errno_location() }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Starting fanotify filesystem monitoring program (Pure unsafe version) ===");
    
    // Check kernel and system support
    check_kernel_version();
    check_capabilities();
    
    // Check if running as root
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    println!("DEBUG: Running as UID: {}, GID: {}", uid, gid);
    
    if uid != 0 {
        eprintln!("WARNING: fanotify typically requires root privileges");
        eprintln!("If you encounter permission errors, try running with sudo");
        eprintln!("Some fanotify features (like FAN_ATTRIB) require CAP_SYS_ADMIN capability");
    }
    
    // Initialize fanotify with raw system call
    println!("DEBUG: Initializing fanotify with FAN_CLASS_NOTIF and O_RDONLY...");
    println!("DEBUG: FAN_CLASS_NOTIF = {}", FAN_CLASS_NOTIF);
    println!("DEBUG: libc::O_RDONLY = {}", libc::O_RDONLY);
    
    let fanotify_fd = unsafe { fanotify_init(FAN_CLASS_NOTIF | FAN_CLOEXEC, libc::O_RDONLY as u32) };
    
    if fanotify_fd == -1 {
        let errno = get_errno();
        eprintln!("✗ Failed to initialize fanotify: errno = {}", errno);
        match errno {
            libc::EPERM => {
                eprintln!("EPERM: Operation not permitted - need root privileges or CAP_SYS_ADMIN");
            }
            libc::ENOSYS => {
                eprintln!("ENOSYS: Function not implemented - fanotify not supported by kernel");
            }
            libc::EINVAL => {
                eprintln!("EINVAL: Invalid argument - check fanotify flags");
            }
            _ => {
                eprintln!("Other error occurred during fanotify initialization: {}", errno);
            }
        }
        return Err(format!("fanotify_init failed with errno {}", errno).into());
    }
    
    println!("✓ Successfully initialized fanotify, fd = {}", fanotify_fd);
    
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
            return Err(e.into());
        }
    }
    
    // Monitor file events - PRIORITIZE METADATA MONITORING (FAN_ATTRIB)
    // FAN_ATTRIB is the MAIN FOCUS - it detects metadata changes like:
    // - chmod (permission changes)
    // - chown (ownership changes) 
    // - utime/utimes (timestamp changes)
    // - truncate (size changes without content modification)
    // - setxattr/removexattr (extended attributes)
    // - link/unlink operations
    let mask_metadata_focused = FAN_ATTRIB | FAN_OPEN | FAN_CLOSE_WRITE;  // Metadata first!
    let mask_fallback = FAN_OPEN | FAN_MODIFY | FAN_CLOSE_WRITE;
    
    println!("=== METADATA MONITORING SETUP ===");
    println!("🎯 PRIMARY GOAL: Monitor file metadata changes (FAN_ATTRIB)");
    println!("DEBUG: mask_metadata_focused = 0x{:x} (ATTRIB priority)", mask_metadata_focused);
    println!("DEBUG: mask_fallback = 0x{:x} (without ATTRIB)", mask_fallback);
    println!("🔧 FAN_ATTRIB monitors these metadata operations:");
    println!("   • chmod/fchmod - Permission changes");
    println!("   • chown/fchown - Ownership changes"); 
    println!("   • utime/utimes - Timestamp modifications");
    println!("   • truncate/ftruncate - Size changes");
    println!("   • setxattr/removexattr - Extended attributes");
    println!("   • link/unlink - Hard link operations");
    println!("DEBUG: Attempting to enable FAN_ATTRIB for metadata monitoring...");
    
    // Convert path to C string
    let path_cstr = std::ffi::CString::new(test_file_path).unwrap();
    
    let mark_result = unsafe {
        fanotify_mark(
            fanotify_fd,
            FAN_MARK_ADD,
            mask_metadata_focused,
            AT_FDCWD,
            path_cstr.as_ptr(),
        )
    };
    
    let actual_mask = if mark_result == -1 {
        let errno = get_errno();
        println!("❌ CRITICAL: Failed to enable FAN_ATTRIB metadata monitoring: errno = {}", errno);
        match errno {
            libc::EINVAL => {
                println!("💔 EINVAL - FAN_ATTRIB is NOT supported on this kernel!");
                println!("   Kernel requirement: Linux 5.1+ for FAN_ATTRIB on files");
                println!("   Current kernel may be too old for metadata monitoring");
            }
            libc::EPERM => {
                println!("🔒 EPERM - Insufficient privileges for FAN_ATTRIB metadata monitoring");
                println!("   FAN_ATTRIB requires CAP_SYS_ADMIN capability");
            }
            _ => {
                println!("🚫 Other error preventing metadata monitoring: errno {}", errno);
            }
        }
        
        println!("⚠️  FALLBACK: Attempting basic monitoring without metadata detection...");
        let basic_result = unsafe {
            fanotify_mark(
                fanotify_fd,
                FAN_MARK_ADD,
                mask_fallback,
                AT_FDCWD,
                path_cstr.as_ptr(),
            )
        };
        
        if basic_result == -1 {
            let errno = get_errno();
            eprintln!("💥 FATAL: Complete failure - cannot even monitor basic file events: errno = {}", errno);
            unsafe { libc::close(fanotify_fd) };
            return Err(format!("fanotify_mark failed completely with errno {}", errno).into());
        }
        
        println!("✅ Fallback successful: Basic file monitoring enabled (NO metadata detection)");
        
        // Try to add directory monitoring for FAN_ATTRIB as additional fallback
        println!("🔍 EXPERIMENTAL: Attempting directory-level FAN_ATTRIB monitoring...");
        let dir_path_cstr = std::ffi::CString::new("/tmp").unwrap();
        let dir_result = unsafe {
            fanotify_mark(
                fanotify_fd,
                FAN_MARK_ADD | FAN_MARK_ONLYDIR,
                FAN_ATTRIB,
                AT_FDCWD,
                dir_path_cstr.as_ptr(),
            )
        };
        
        if dir_result == 0 {
            println!("✨ SUCCESS: Directory-level FAN_ATTRIB monitoring enabled!");
            println!("   This may detect some metadata changes at directory level");
            mask_fallback | FAN_ATTRIB
        } else {
            println!("❌ Directory-level FAN_ATTRIB also failed: errno = {}", get_errno());
            mask_fallback
        }
    } else {
        println!("🎉 SUCCESS: FAN_ATTRIB metadata monitoring is ACTIVE!");
        println!("✨ This system fully supports metadata change detection");
        mask_metadata_focused
    };
    
    println!("\n🎯 === METADATA MONITORING STATUS ===");
    if actual_mask & FAN_ATTRIB != 0 {
        println!("✅ METADATA MONITORING: ✨ FULLY ACTIVE ✨");
        println!("📊 Event types being monitored:");
        println!("   🔧 ATTRIB (metadata) ← PRIMARY TARGET");
        println!("   🔓 OPEN (file access)");
        println!("   💾 CLOSE_WRITE (write completion)");
        println!("🎉 SUCCESS: All metadata changes will be detected!");
        println!("   • Permission changes (chmod): ✅ WILL DETECT");
        println!("   • Ownership changes (chown): ✅ WILL DETECT");
        println!("   • Timestamp changes (touch): ✅ WILL DETECT");
        println!("   • Size changes (truncate): ✅ WILL DETECT");
        println!("   • Extended attributes: ✅ WILL DETECT");
    } else {
        println!("❌ METADATA MONITORING: 💔 NOT AVAILABLE 💔");
        println!("📊 Event types being monitored (limited):");
        println!("   🔓 OPEN (file access)");
        println!("   📝 MODIFY (content changes)");
        println!("   💾 CLOSE_WRITE (write completion)");
        println!("⚠️  WARNING: Metadata changes will NOT be detected!");
        println!("   • Permission changes (chmod): ❌ WILL NOT DETECT");
        println!("   • Ownership changes (chown): ❌ WILL NOT DETECT");  
        println!("   • Timestamp changes (touch): ❌ WILL NOT DETECT");
        println!("   • Only content modifications will be visible");
    }
    println!("Monitoring file: {}", test_file_path);
    println!("Press Ctrl+C to exit the program");
    println!("\n🧪 === METADATA TESTING COMMANDS ===");
    println!("💡 Run these commands in another terminal to test metadata monitoring:");
    println!("\n🔧 METADATA CHANGE TESTS (should trigger FAN_ATTRIB if supported):");
    println!("   chmod 755 {} # Change permissions", test_file_path);
    println!("   chmod 644 {} # Restore permissions", test_file_path);
    println!("   touch {} # Update timestamps", test_file_path);
    println!("   chown $USER:$USER {} # Change ownership", test_file_path);
    println!("   truncate -s 100 {} # Change file size", test_file_path);
    println!("   truncate -s 0 {} # Truncate to empty", test_file_path);
    if actual_mask & FAN_ATTRIB != 0 {
        println!("   ✨ These commands WILL generate FAN_ATTRIB events!");
    } else {
        println!("   ⚠️  These commands will NOT be detected (FAN_ATTRIB unavailable)");
    }
    println!("\n📝 Content modification tests (for comparison):");
    println!("   echo 'new content' >> {}", test_file_path);
    println!("   cat {} > /dev/null", test_file_path);
    if actual_mask & FAN_ATTRIB == 0 {
        println!("   ✅ These commands WILL be detected with basic monitoring");
    }
    println!("=== End Test Commands ===\n");
    
    if actual_mask & FAN_ATTRIB != 0 {
        println!("🎯 READY: Waiting for METADATA CHANGES (FAN_ATTRIB events)...\n");
    } else {
        println!("⚠️  READY: Waiting for file events (metadata changes will be missed)...\n");
    }

    let mut event_count = 0;
    println!("DEBUG: Entering event loop, waiting for fanotify events...");
    
    // Event buffer
    const BUF_SIZE: usize = 4096;
    let mut buffer = [0u8; BUF_SIZE];
    
    loop {
        println!("DEBUG: Calling read() on fanotify fd...");
        let bytes_read = unsafe {
            libc::read(fanotify_fd, buffer.as_mut_ptr() as *mut libc::c_void, BUF_SIZE)
        };
        
        if bytes_read == -1 {
            let errno = get_errno();
            match errno {
                libc::EINTR => {
                    println!("DEBUG: EINTR - Interrupted system call, this is normal");
                    continue;
                }
                libc::EAGAIN => {
                    println!("DEBUG: EAGAIN - No events available right now");
                    continue;
                }
                _ => {
                    eprintln!("✗ Error reading fanotify events: errno = {}", errno);
                    break;
                }
            }
        }
        
        if bytes_read == 0 {
            println!("DEBUG: read() returned 0, continuing...");
            continue;
        }
        
        println!("DEBUG: Read {} bytes from fanotify", bytes_read);
        
        // Parse events from buffer
        let mut offset = 0;
        while offset < bytes_read as usize {
            if offset + mem::size_of::<FanotifyEventMetadata>() > bytes_read as usize {
                println!("DEBUG: Incomplete event data, breaking");
                break;
            }
            
            let event: &FanotifyEventMetadata = unsafe {
                &*(buffer.as_ptr().add(offset) as *const FanotifyEventMetadata)
            };
            
            event_count += 1;
            println!("\n=== EVENT #{} ===", event_count);
            println!("DEBUG: Raw event: {:?}", event);
            println!("DEBUG: Event mask: 0x{:x}", event.mask);
            println!("DEBUG: Event PID: {}", event.pid);
            println!("DEBUG: Event FD: {}", event.fd);
            
            // Decode individual mask flags with METADATA EMPHASIS
            println!("🎯 METADATA FOCUS - Mask flag analysis:");
            println!("  🔧 FAN_ATTRIB (METADATA): {}", if event.mask & FAN_ATTRIB != 0 { "🎉 YES!" } else { "❌ No" });
            println!("  🔓 FAN_OPEN: {}", event.mask & FAN_OPEN != 0);
            println!("  📝 FAN_MODIFY: {}", event.mask & FAN_MODIFY != 0);
            println!("  💾 FAN_CLOSE_WRITE: {}", event.mask & FAN_CLOSE_WRITE != 0);
            
            // Get file path
            let path_info = if event.fd >= 0 {
                println!("DEBUG: Event has file descriptor: {}", event.fd);
                let link = format!("/proc/self/fd/{}", event.fd);
                println!("DEBUG: Attempting to resolve path via: {}", link);
                match fs::read_link(&link) {
                    Ok(path) => {
                        let path_str = path.display().to_string();
                        println!("DEBUG: ✓ Resolved path: {}", path_str);
                        format!("path={}", path_str)
                    },
                    Err(e) => {
                        println!("DEBUG: ✗ Failed to resolve path for fd {}: {}", event.fd, e);
                        format!("fd={}", event.fd)
                    },
                }
            } else {
                println!("DEBUG: ⚠ Invalid file descriptor in event: {}", event.fd);
                "path=unknown".to_string()
            };

            // Print event summary with METADATA PRIORITY
            println!("\n🎯 EVENT SUMMARY (Metadata Focus):");
            let mut event_types = Vec::new();
            
            // CHECK FOR METADATA CHANGES FIRST (highest priority)
            if event.mask & FAN_ATTRIB != 0 {
                println!("🎉 � [ATTRIB - METADATA CHANGE!] pid={} {} - File metadata/attributes modified!", event.pid, path_info);
                println!("   🎯 METADATA CHANGE DETECTED! This could be:");
                println!("   • 🔐 chmod/fchmod (permission changes)");
                println!("   • 👤 chown/fchown (ownership changes)");
                println!("   • ⏰ utime/utimes (timestamp modifications)");
                println!("   • ✂️  truncate/ftruncate (size changes without content write)");
                println!("   • 🏷️  setxattr/removexattr (extended attributes)");
                println!("   • 🔗 link/unlink operations");
                event_types.push("🔧 ATTRIB-METADATA");
            }
            
            // Other events (secondary priority)
            if event.mask & FAN_OPEN != 0 {
                println!("� [OPEN] pid={} {} - File opened for reading/writing", event.pid, path_info);
                event_types.push("OPEN");
            }
            if event.mask & FAN_MODIFY != 0 {
                println!("� [MODIFY] pid={} {} - File content was modified", event.pid, path_info);
                event_types.push("MODIFY");
            }
            if event.mask & FAN_CLOSE_WRITE != 0 {
                println!("� [CLOSE_WRITE] pid={} {} - Writable file was closed", event.pid, path_info);
                event_types.push("CLOSE_WRITE");
            }
            
            if event_types.is_empty() {
                println!("❓ [UNKNOWN] pid={} {} - Unrecognized event type (mask: 0x{:x})", event.pid, path_info, event.mask);
            } else if event.mask & FAN_ATTRIB != 0 {
                println!("🎯 ⭐ METADATA EVENT PRIORITY: This is exactly what we're looking for!");
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
            
            // Close the event file descriptor
            if event.fd >= 0 {
                unsafe { libc::close(event.fd) };
                println!("DEBUG: Closed event file descriptor {}", event.fd);
            }
            
            println!("==========================================");
            
            // Move to next event
            offset += event.event_len as usize;
        }
    }
    
    // Clean up
    unsafe { libc::close(fanotify_fd) };
    println!("DEBUG: Closed fanotify file descriptor");
    
    Ok(())
}
