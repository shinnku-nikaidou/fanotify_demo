use nix::sys::fanotify::{EventFFlags, Fanotify, InitFlags, MarkFlags, MaskFlags};
use std::{fs, os::unix::io::AsRawFd, path::Path};

fn main() -> nix::Result<()> {
    println!("Starting fanotify filesystem monitoring program...");
    
    // Initialize fanotify with basic settings
    let fan = Fanotify::init(
        InitFlags::FAN_CLASS_NOTIF,
        EventFFlags::O_RDONLY,
    )?;

    println!("Successfully initialized fanotify");

    // Monitor only file attribute change events
    let mask = MaskFlags::FAN_ATTRIB;

    // Monitor /tmp directory (safer test directory)
    fan.mark(
        MarkFlags::FAN_MARK_ADD,
        mask,
        std::io::stdin(),
        Some(Path::new("/tmp")),
    )?;

    println!("Starting to listen for filesystem events...");
    println!("Event types being monitored: ATTRIB, MODIFY, OPEN, CLOSE_WRITE, CREATE, DELETE");
    println!("Press Ctrl+C to exit the program");

    loop {
        for ev in fan.read_events()? {
            let mask = ev.mask();
            let pid = ev.pid();
            
            // Get file path
            let path_info = if let Some(fd) = ev.fd() {
                let link = format!("/proc/self/fd/{}", fd.as_raw_fd());
                match fs::read_link(&link) {
                    Ok(path) => format!("path={}", path.display()),
                    Err(_) => format!("fd={}", fd.as_raw_fd()),
                }
            } else {
                "path=unknown".to_string()
            };

            // Print different information based on event type
            if mask.contains(MaskFlags::FAN_ATTRIB) {
                println!("[ATTRIB] pid={} {} - File attributes/metadata changed", pid, path_info);
            }
            if mask.contains(MaskFlags::FAN_MODIFY) {
                println!("[MODIFY] pid={} {} - File content modified", pid, path_info);
            }
            if mask.contains(MaskFlags::FAN_OPEN) {
                println!("[OPEN] pid={} {} - File opened", pid, path_info);
            }
            if mask.contains(MaskFlags::FAN_CLOSE_WRITE) {
                println!("[CLOSE_WRITE] pid={} {} - Writable file closed", pid, path_info);
            }
            if mask.contains(MaskFlags::FAN_CREATE) {
                println!("[CREATE] pid={} {} - File/directory created", pid, path_info);
            }
            if mask.contains(MaskFlags::FAN_DELETE) {
                println!("[DELETE] pid={} {} - File/directory deleted", pid, path_info);
            }
        }
    }
}
