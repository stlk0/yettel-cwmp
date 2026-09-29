use std::{
    fs::File,
    io::{self, Read},
    os::fd::{AsRawFd, FromRawFd},
};

pub struct Pty {
    pub master: File,
    pub slave: File,
    original: libc::termios,
}
impl Pty {
    pub fn new(rows: u16, columns: u16) -> Self {
        let mut master = -1;
        let mut slave = -1;
        let mut size = libc::winsize {
            ws_row: rows,
            ws_col: columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        assert_eq!(
            // SAFETY: all output pointers point to initialized writable locals; size is valid.
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::addr_of_mut!(size),
                )
            },
            0,
            "openpty: {}",
            io::Error::last_os_error()
        );
        // SAFETY: openpty returned two owned, valid fds; ownership moves into Files once.
        let master = unsafe { File::from_raw_fd(master) };
        // SAFETY: the slave fd is distinct and still owned by this function.
        let slave = unsafe { File::from_raw_fd(slave) };
        for file in [&master, &slave] {
            assert_ne!(
                // SAFETY: file owns a valid fd; F_SETFD accepts this integer flag.
                unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) },
                -1
            );
        }
        // SAFETY: master owns a valid fd; F_GETFL takes no pointer argument.
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        assert_ne!(flags, -1);
        assert_ne!(
            // SAFETY: master owns a valid fd and flags came from F_GETFL above.
            unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
            -1
        );
        let original = modes(&slave);
        Self {
            master,
            slave,
            original,
        }
    }
    pub fn read_available(&mut self, output: &mut Vec<u8>) {
        let mut buffer = [0; 8192];
        loop {
            match self.master.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => output.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.raw_os_error() == Some(libc::EIO) =>
                {
                    break;
                }
                Err(error) => panic!("PTY read failed: {error}"),
            }
        }
    }
    pub fn assert_restored(&self, output: &[u8]) {
        let after = modes(&self.slave);
        let signature = |mode: &libc::termios| {
            (
                mode.c_iflag,
                mode.c_oflag,
                mode.c_cflag,
                mode.c_lflag,
                mode.c_cc,
                // SAFETY: mode is a valid termios value returned by tcgetattr.
                unsafe { libc::cfgetispeed(mode) },
                // SAFETY: mode is a valid termios value returned by tcgetattr.
                unsafe { libc::cfgetospeed(mode) },
            )
        };
        assert_eq!(
            signature(&self.original),
            signature(&after),
            "terminal modes were not restored"
        );
        for escape in [b"\x1b[?1049l".as_slice(), b"\x1b[?25h", b"\x1b[?2004l"] {
            assert!(
                output.windows(escape.len()).any(|part| part == escape),
                "terminal restoration sequence missing: {escape:?}"
            );
        }
    }
}
fn modes(file: &File) -> libc::termios {
    #[cfg(target_os = "macos")]
    {
        let mut pending = 0 as libc::c_int;
        // Darwin sets PENDIN while restoring canonical input. Querying pending bytes
        // settles that kernel state without consuming input or masking a real mode error.
        assert_eq!(
            // SAFETY: `pending` is a valid writable integer and `file` owns this PTY fd.
            unsafe { libc::ioctl(file.as_raw_fd(), libc::FIONREAD, &mut pending) },
            0
        );
    }
    // SAFETY: termios is a C POD value and tcgetattr initializes every field.
    let mut modes = unsafe { std::mem::zeroed() };
    // SAFETY: file owns a valid PTY fd; modes is a writable termios value.
    assert_eq!(unsafe { libc::tcgetattr(file.as_raw_fd(), &mut modes) }, 0);
    modes
}
