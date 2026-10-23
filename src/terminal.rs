use std::io::{self, Write};
use std::mem;
use std::time::Duration;

const ENTER_SCREEN: &str = "\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1006h";
const LEAVE_SCREEN: &str = "\x1b[?1006l\x1b[?1000l\x1b[?25h\x1b[?1049l";

#[derive(Debug, PartialEq, Eq)]
pub enum Input {
    Up,
    Down,
    Enter,
    Refresh,
    Quit,
    Click { row: usize },
}

pub struct Terminal {
    original: libc::termios,
}

impl Terminal {
    pub fn enter() -> io::Result<Terminal> {
        let mut original: libc::termios = unsafe { mem::zeroed() };
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut original) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = original;
        unsafe { libc::cfmakeraw(&mut raw) };
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut out = io::stdout();
        out.write_all(ENTER_SCREEN.as_bytes())?;
        out.flush()?;
        Ok(Terminal { original })
    }

    pub fn size(&self) -> (usize, usize) {
        let mut size: libc::winsize = unsafe { mem::zeroed() };
        if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } != 0 {
            return (40, 24);
        }
        (size.ws_col as usize, size.ws_row as usize)
    }

    pub fn read_input(&self, timeout: Duration) -> io::Result<Vec<Input>> {
        let mut poll_fd = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut poll_fd, 1, timeout.as_millis() as libc::c_int) };
        if ready <= 0 {
            return Ok(Vec::new());
        }
        let mut buffer = [0u8; 1024];
        let read =
            unsafe { libc::read(libc::STDIN_FILENO, buffer.as_mut_ptr().cast(), buffer.len()) };
        if read <= 0 {
            return Ok(vec![Input::Quit]);
        }
        Ok(parse_input(&buffer[..read as usize]))
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = out.write_all(LEAVE_SCREEN.as_bytes());
        let _ = out.flush();
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original) };
    }
}

fn parse_mouse(sequence: &[u8]) -> Option<(Option<Input>, usize)> {
    let end = sequence
        .iter()
        .position(|&byte| byte == b'M' || byte == b'm')?;
    let fields: Vec<usize> = std::str::from_utf8(&sequence[3..end])
        .ok()?
        .split(';')
        .map(|field| field.parse().ok())
        .collect::<Option<_>>()?;
    let input = match (fields.as_slice(), sequence[end]) {
        ([0, _, row], b'M') => Some(Input::Click {
            row: row.saturating_sub(1),
        }),
        _ => None,
    };
    Some((input, end + 1))
}

pub fn parse_input(mut data: &[u8]) -> Vec<Input> {
    let mut inputs = Vec::new();
    while !data.is_empty() {
        if data.starts_with(b"\x1b[<") {
            match parse_mouse(data) {
                Some((input, consumed)) => {
                    inputs.extend(input);
                    data = &data[consumed..];
                    continue;
                }
                None => break,
            }
        }
        if data.starts_with(b"\x1b[A") || data.starts_with(b"\x1bOA") {
            inputs.push(Input::Up);
            data = &data[3..];
            continue;
        }
        if data.starts_with(b"\x1b[B") || data.starts_with(b"\x1bOB") {
            inputs.push(Input::Down);
            data = &data[3..];
            continue;
        }
        let (key, rest) = (data[0], &data[1..]);
        data = rest;
        match key {
            b'q' | 0x03 => inputs.push(Input::Quit),
            0x1b if data.is_empty() => inputs.push(Input::Quit),
            b'j' => inputs.push(Input::Down),
            b'k' => inputs.push(Input::Up),
            b'\r' | b'\n' | b'l' => inputs.push(Input::Enter),
            b'r' => inputs.push(Input::Refresh),
            _ => {}
        }
    }
    inputs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_keys_arrows_and_clicks() {
        assert_eq!(
            parse_input(b"jk\r"),
            vec![Input::Down, Input::Up, Input::Enter]
        );
        assert_eq!(parse_input(b"\x1b[A\x1b[B"), vec![Input::Up, Input::Down]);
        assert_eq!(
            parse_input(b"\x1b[<0;5;3M\x1b[<0;5;3m"),
            vec![Input::Click { row: 2 }]
        );
        assert_eq!(parse_input(b"\x1b"), vec![Input::Quit]);
        assert_eq!(parse_input(b"\x1b[<35;1;1M"), Vec::<Input>::new());
    }
}
