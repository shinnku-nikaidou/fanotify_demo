use nix::sys::fanotify::{EventFFlags, Fanotify, InitFlags, MarkFlags, MaskFlags};
use std::{fs, os::unix::io::AsRawFd};
use nix::unistd::close;

fn main() -> nix::Result<()> {
    println!("Starting fanotify filesystem monitoring program...");
    
    // Initialize fanotify with basic settings
    let fan = Fanotify::init(
        InitFlags::FAN_CLASS_NOTIF,
        EventFFlags::O_RDONLY,
    )?;

    println!("Successfully initialized fanotify");

    // Monitor file attribute and metadata change events - start with basic events
    let mask = MaskFlags::FAN_OPEN | MaskFlags::FAN_MODIFY | MaskFlags::FAN_CLOSE_WRITE;

    // Monitor /tmp directory - open the directory first to get a proper file descriptor
    let tmp_dir = match std::fs::File::open("/tmp") {
        Ok(file) => file,
        Err(e) => {
            eprintln!("Failed to open /tmp directory: {}", e);
            return Err(nix::errno::Errno::ENOENT);
        }
    };
    match fan.mark(
        MarkFlags::FAN_MARK_ADD,
        mask,
        &tmp_dir,
        None::<&std::path::Path>, // Use None when providing a file descriptor
    ) {
        Ok(_) => println!("Successfully marked /tmp directory for monitoring"),
        Err(e) => {
            eprintln!("Failed to mark /tmp directory: {}", e);
            return Err(e);
        }
    }

    println!("Starting to listen for filesystem events...");
    println!("Event types being monitored: OPEN, MODIFY, CLOSE_WRITE");
    println!("Press Ctrl+C to exit the program");

    loop {
        for ev in fan.read_events()? {
            let mask = ev.mask();
            let pid = ev.pid();
            
            // Get file path and close file descriptor properly
            let path_info = if let Some(fd) = ev.fd() {
                let fd_raw = fd.as_raw_fd();
                let link = format!("/proc/self/fd/{}", fd_raw);
                let result = match fs::read_link(&link) {
                    Ok(path) => format!("path={}", path.display()),
                    Err(_) => format!("fd={}", fd_raw),
                };
                // Close the file descriptor to avoid leaking
                let _ = close(fd_raw);
                result
            } else {
                "path=unknown".to_string()
            };

            // Print different information based on event type
            if mask.contains(MaskFlags::FAN_OPEN) {
                println!("[OPEN] pid={} {} - File opened", pid, path_info);
            }
            if mask.contains(MaskFlags::FAN_MODIFY) {
                println!("[MODIFY] pid={} {} - File content modified", pid, path_info);
            }
            if mask.contains(MaskFlags::FAN_CLOSE_WRITE) {
                println!("[CLOSE_WRITE] pid={} {} - Writable file closed", pid, path_info);
            }
        }
    }
}
