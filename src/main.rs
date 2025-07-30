use nix::sys::fanotify::{EventFFlags, Fanotify, InitFlags, MarkFlags, MaskFlags};
use std::{fs, os::unix::io::AsRawFd, path::Path};
use nix::unistd::close;

fn main() -> nix::Result<()> {
    println!("Starting fanotify filesystem monitoring program...");
    
    // Initialize fanotify with basic settings
    let fan = Fanotify::init(
        InitFlags::FAN_CLASS_NOTIF,
        EventFFlags::O_RDONLY,
    )?;

    println!("Successfully initialized fanotify");

    // Monitor file attribute and metadata change events
    let mask = MaskFlags::FAN_ATTRIB | MaskFlags::FAN_MODIFY | MaskFlags::FAN_CREATE | MaskFlags::FAN_DELETE;

    // Monitor /tmp directory - pass the fanotify instance itself as fd for directory monitoring
    fan.mark(
        MarkFlags::FAN_MARK_ADD,
        mask,
        &fan,
        Some(Path::new("/tmp")),
    )?;

    println!("Starting to listen for filesystem events...");
    println!("Event types being monitored: ATTRIB, MODIFY, CREATE, DELETE");
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
            if mask.contains(MaskFlags::FAN_ATTRIB) {
                println!("[ATTRIB] pid={} {} - File attributes/metadata changed", pid, path_info);
            }
            if mask.contains(MaskFlags::FAN_MODIFY) {
                println!("[MODIFY] pid={} {} - File content modified", pid, path_info);
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
