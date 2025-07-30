use nix::sys::fanotify::{EventFFlags, Fanotify, InitFlags, MarkFlags, MaskFlags};
use std::{fs, os::unix::io::AsRawFd, path::Path};

fn main() -> nix::Result<()> {
    let fan = Fanotify::init(
        InitFlags::empty(),
        EventFFlags::O_CLOEXEC | EventFFlags::O_RDONLY,
    )?;

    // 打开当前目录作为文件描述符
    let cwd = std::fs::File::open(".").map_err(|e| nix::errno::Errno::from_raw(e.raw_os_error().unwrap_or(0)))?;
    fan.mark(
        MarkFlags::FAN_MARK_ADD,
        MaskFlags::FAN_ATTRIB,
        &cwd,
        Some(Path::new("/home")),
    )?;

    loop {
        for ev in fan.read_events()? {
            if !ev.mask().contains(MaskFlags::FAN_ATTRIB) {
                continue;
            }
            if let Some(fd) = ev.fd() {
                let link = format!("/proc/self/fd/{}", fd.as_raw_fd());
                if let Ok(path) = fs::read_link(&link) {
                    println!("[ATTRIB] pid={} path={}", ev.pid(), path.display());
                }
            }
        }
    }
}
