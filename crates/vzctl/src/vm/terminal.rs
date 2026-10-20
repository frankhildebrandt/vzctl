use super::*;

pub(super) fn terminal_winsize() -> (u16, u16) {
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } == 0;
    let cols = if ok && size.ws_col > 0 {
        size.ws_col
    } else {
        80
    };
    let rows = if ok && size.ws_row > 0 {
        size.ws_row
    } else {
        24
    };
    (cols, rows)
}

pub(super) const MUX_STDIN: u8 = 0x01;
pub(super) const MUX_STDOUT: u8 = 0x02;
pub(super) const MUX_RESIZE: u8 = 0x04;
pub(super) const MUX_EXIT: u8 = 0x05;
pub(super) const MUX_STDIN_EOF: u8 = 0x06;

pub(super) fn write_mux_frame(
    stream: &mut UnixStream,
    frame_type: u8,
    payload: &[u8],
) -> Result<(), Failure> {
    use std::io::Write as IoWrite;
    if payload.len() > 1_048_576 {
        return Err(Failure::new(EXIT_VM_OP, "mux frame exceeds 1 MiB"));
    }
    let mut header = [0_u8; 5];
    header[0] = frame_type;
    header[1..5].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    stream
        .write_all(&header)
        .and_then(|_| {
            if payload.is_empty() {
                Ok(())
            } else {
                stream.write_all(payload)
            }
        })
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("mux write: {error}")))
}

pub(super) fn mux_tty_session(stream: &mut UnixStream) -> Result<u8, Failure> {
    use std::io::{Read, Write as IoWrite};
    use std::os::fd::AsRawFd;

    let stdin_fd = libc::STDIN_FILENO;
    let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
    let had_tty = unsafe { libc::tcgetattr(stdin_fd, original.as_mut_ptr()) } == 0;
    let original = if had_tty {
        Some(unsafe { original.assume_init() })
    } else {
        None
    };
    if let Some(mut raw) = original {
        unsafe { libc::cfmakeraw(&mut raw) };
        if unsafe { libc::tcsetattr(stdin_fd, libc::TCSANOW, &raw) } != 0 {
            return Err(Failure::new(EXIT_VM_OP, "cannot set raw terminal mode"));
        }
    }
    let _restore = TerminalRestore(original);

    stream
        .set_nonblocking(true)
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, error.to_string()))?;
    let mut stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut detach = ConsoleDetachState::default();
    let mut exit_code = 0_u8;
    loop {
        match read_mux_frame_nonblocking(stream) {
            Ok(None) => {}
            Ok(Some((MUX_STDOUT, payload))) => {
                stdout
                    .write_all(&payload)
                    .and_then(|_| stdout.flush())
                    .map_err(|error| Failure::new(EXIT_VM_OP, error.to_string()))?;
            }
            Ok(Some((MUX_EXIT, payload))) => {
                if payload.len() == 4 {
                    let status = i32::from_le_bytes(payload[0..4].try_into().unwrap());
                    exit_code = status.clamp(0, 255) as u8;
                }
                break;
            }
            Ok(Some(_)) => {
                return Err(Failure::new(EXIT_SUPERVISOR, "unexpected mux frame"));
            }
            Err(error) if error.message.contains("WouldBlock") => {}
            Err(error) => return Err(error),
        }

        let mut pollfd = libc::pollfd {
            fd: stdin_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut pollfd, 1, 50) };
        if ready > 0 && pollfd.revents & libc::POLLIN != 0 {
            let mut input = [0_u8; 1024];
            match stdin.read(&mut input) {
                Ok(0) => {
                    write_mux_frame(stream, MUX_STDIN_EOF, &[])?;
                }
                Ok(count) => {
                    let mut forward = Vec::new();
                    if consume_console_stdin(&mut detach, &input[..count], &mut forward) {
                        break;
                    }
                    if !forward.is_empty() {
                        write_mux_frame(stream, MUX_STDIN, &forward)?;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => {
                    return Err(Failure::new(EXIT_VM_OP, format!("stdin read: {error}")));
                }
            }
        }

        let mut sock_poll = libc::pollfd {
            fd: stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let _ = unsafe { libc::poll(&mut sock_poll, 1, 0) };
        if sock_poll.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            break;
        }
    }
    Ok(exit_code)
}

pub(super) fn read_mux_frame_nonblocking(
    stream: &mut UnixStream,
) -> Result<Option<(u8, Vec<u8>)>, Failure> {
    use std::io::Read;
    let mut header = [0_u8; 5];
    match stream.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(Failure::new(EXIT_SUPERVISOR, "mux connection closed"));
        }
        Err(error) => {
            return Err(Failure::new(EXIT_SUPERVISOR, format!("mux read: {error}")));
        }
    }
    let length = u32::from_le_bytes(header[1..5].try_into().unwrap()) as usize;
    if length > 1_048_576 {
        return Err(Failure::new(EXIT_SUPERVISOR, "mux frame exceeds 1 MiB"));
    }
    let mut payload = vec![0_u8; length];
    if length > 0 {
        // After header, block briefly for payload (socket may be nonblocking).
        stream
            .set_nonblocking(false)
            .map_err(|error| Failure::new(EXIT_SUPERVISOR, error.to_string()))?;
        let result = stream.read_exact(&mut payload);
        stream
            .set_nonblocking(true)
            .map_err(|error| Failure::new(EXIT_SUPERVISOR, error.to_string()))?;
        result.map_err(|error| Failure::new(EXIT_SUPERVISOR, format!("mux read: {error}")))?;
    }
    Ok(Some((header[0], payload)))
}

pub(super) fn attach_vm(id: &str) -> Result<Value, Failure> {
    validate_vm_id(id)?;
    let path = console_socket_path(id);
    let mut stream = UnixStream::connect(&path).map_err(|error| {
        Failure::new(
            EXIT_SUPERVISOR,
            format!(
                "console socket {}: {error} (is the VM running?)",
                path.display()
            ),
        )
    })?;
    eprintln!("attached to {id} serial console (Ctrl-P Ctrl-Q to detach)");
    raw_console_session(&mut stream)?;
    Ok(json!({
        "apiVersion": API_VERSION,
        "command": "vm.attach",
        "status": "ok",
        "exit_code": 0,
        "summary": {
            "message": format!("detached from {id}"),
            "vm_id": id,
        },
    }))
}

pub(super) fn console_socket_path(id: &str) -> PathBuf {
    crate::state_dir()
        .join("helpers")
        .join(format!("{}.console.sock", socket_file_component(id)))
}

pub(super) fn state_file_component(value: &str) -> String {
    let prefix: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    format!("{prefix}-{}", socket_file_component(value))
}

pub(super) fn socket_file_component(value: &str) -> String {
    let hash = value
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("{hash:x}")
}

pub(super) fn raw_console_session(stream: &mut UnixStream) -> Result<(), Failure> {
    use std::io::{Read, Write as IoWrite};
    use std::os::fd::AsRawFd;

    let stdin_fd = libc::STDIN_FILENO;
    let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
    let had_tty = unsafe { libc::tcgetattr(stdin_fd, original.as_mut_ptr()) } == 0;
    let original = if had_tty {
        Some(unsafe { original.assume_init() })
    } else {
        None
    };
    if let Some(mut raw) = original {
        unsafe { libc::cfmakeraw(&mut raw) };
        if unsafe { libc::tcsetattr(stdin_fd, libc::TCSANOW, &raw) } != 0 {
            return Err(Failure::new(EXIT_VM_OP, "cannot set raw terminal mode"));
        }
    }
    let _restore = TerminalRestore(original);

    stream
        .set_nonblocking(true)
        .map_err(|error| Failure::new(EXIT_SUPERVISOR, error.to_string()))?;
    let mut stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut buffer = [0_u8; 4096];
    let mut detach = ConsoleDetachState::default();
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                stdout
                    .write_all(&buffer[..count])
                    .and_then(|_| stdout.flush())
                    .map_err(|error| Failure::new(EXIT_VM_OP, error.to_string()))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => {
                return Err(Failure::new(
                    EXIT_SUPERVISOR,
                    format!("console read: {error}"),
                ));
            }
        }

        let mut pollfd = libc::pollfd {
            fd: stdin_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut pollfd, 1, 50) };
        if ready > 0 && pollfd.revents & libc::POLLIN != 0 {
            let mut input = [0_u8; 1024];
            match stdin.read(&mut input) {
                Ok(0) => break,
                Ok(count) => {
                    let mut forward = Vec::new();
                    if consume_console_stdin(&mut detach, &input[..count], &mut forward) {
                        if !forward.is_empty() {
                            stream.write_all(&forward).map_err(|error| {
                                Failure::new(EXIT_SUPERVISOR, format!("console write: {error}"))
                            })?;
                        }
                        break;
                    }
                    if !forward.is_empty() {
                        stream.write_all(&forward).map_err(|error| {
                            Failure::new(EXIT_SUPERVISOR, format!("console write: {error}"))
                        })?;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => {
                    return Err(Failure::new(EXIT_VM_OP, format!("stdin read: {error}")));
                }
            }
        }

        let mut sock_poll = libc::pollfd {
            fd: stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let _ = unsafe { libc::poll(&mut sock_poll, 1, 0) };
        if sock_poll.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            break;
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ConsoleDetachState {
    pub(super) saw_ctrl_p: bool,
}

/// Returns `true` when the Docker-style detach sequence was completed.
pub(super) fn consume_console_stdin(
    state: &mut ConsoleDetachState,
    input: &[u8],
    forward: &mut Vec<u8>,
) -> bool {
    for &byte in input {
        if state.saw_ctrl_p {
            state.saw_ctrl_p = false;
            if byte == CONSOLE_CTRL_Q {
                return true;
            }
            forward.push(CONSOLE_CTRL_P);
            if byte != CONSOLE_CTRL_P {
                forward.push(byte);
            }
            continue;
        }
        if byte == CONSOLE_CTRL_P {
            state.saw_ctrl_p = true;
            continue;
        }
        forward.push(byte);
    }
    false
}

pub(super) struct TerminalRestore(pub(super) Option<libc::termios>);

impl Drop for TerminalRestore {
    fn drop(&mut self) {
        if let Some(termios) = self.0 {
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &termios);
            }
        }
    }
}
