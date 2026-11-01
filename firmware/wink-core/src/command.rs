//! Text commands written by the phone to the command characteristic.
//!
//! Grammar (case-insensitive, tokens separated by spaces, surrounding
//! whitespace, CR/LF and NUL bytes ignored):
//!
//! | Command          | Meaning                                            |
//! |------------------|----------------------------------------------------|
//! | `WINK L`         | wink the LH lamp (`LEFT`, `LH` also accepted)      |
//! | `WINK R`         | wink the RH lamp (`RIGHT`, `RH`)                   |
//! | `WINK B`         | wink both (`BOTH`)                                 |
//! | `ALT [n]`        | alternate for n seconds (default 8, clamped to 10) |
//! | `STOP`           | all relays off now                                 |
//! | `PAIR REMOTE`    | scan for a BLE HID remote and bond with it         |
//! | `FORGET REMOTE`  | delete the stored remote                           |
//! | `STATUS`         | re-send the current state on the notify channel    |

/// Which lamp(s) a wink applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Both,
}

impl Side {
    pub const fn lh(self) -> bool {
        matches!(self, Side::Left | Side::Both)
    }
    pub const fn rh(self) -> bool {
        matches!(self, Side::Right | Side::Both)
    }
    pub const fn letter(self) -> &'static str {
        match self {
            Side::Left => "L",
            Side::Right => "R",
            Side::Both => "B",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Wink(Side),
    /// Alternate. `secs` is as requested; the controller clamps it.
    Alt { secs: u32 },
    Stop,
    PairRemote,
    ForgetRemote,
    Status,
    /// Refuse lamp movement while the engine runs (stored on the module).
    DriveLock(bool),
}

impl Command {
    /// Does this command move a lamp? (What the drive lock refuses; STOP never.)
    pub const fn moves_lamps(self) -> bool {
        matches!(self, Command::Wink(_) | Command::Alt { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    TooLong,
    NotAscii,
    Unknown,
    BadArgument,
}

impl ParseError {
    pub const fn as_str(self) -> &'static str {
        match self {
            ParseError::Empty => "empty",
            ParseError::TooLong => "too long",
            ParseError::NotAscii => "not ascii",
            ParseError::Unknown => "unknown command",
            ParseError::BadArgument => "bad argument",
        }
    }
}

/// Longest accepted command, in bytes after trimming.
pub const MAX_LEN: usize = 32;

/// Parse one command. `alt_default_s` is used for a bare `ALT`.
pub fn parse(input: &[u8], alt_default_s: u32) -> Result<Command, ParseError> {
    let trimmed = trim(input);
    if trimmed.is_empty() {
        return Err(ParseError::Empty);
    }
    if trimmed.len() > MAX_LEN {
        return Err(ParseError::TooLong);
    }
    if !trimmed.is_ascii() {
        return Err(ParseError::NotAscii);
    }
    let mut buf = [0u8; MAX_LEN];
    let buf = &mut buf[..trimmed.len()];
    buf.copy_from_slice(trimmed);
    buf.make_ascii_uppercase();
    // Safe: ASCII checked above.
    let text = core::str::from_utf8(buf).map_err(|_| ParseError::NotAscii)?;

    let mut tokens = text.split_ascii_whitespace();
    let verb = tokens.next().ok_or(ParseError::Empty)?;
    let arg = tokens.next();
    if tokens.next().is_some() {
        return Err(ParseError::BadArgument);
    }

    match (verb, arg) {
        ("WINK", Some(side)) => match side {
            "L" | "LEFT" | "LH" => Ok(Command::Wink(Side::Left)),
            "R" | "RIGHT" | "RH" => Ok(Command::Wink(Side::Right)),
            "B" | "BOTH" => Ok(Command::Wink(Side::Both)),
            _ => Err(ParseError::BadArgument),
        },
        ("WINK", None) => Err(ParseError::BadArgument),
        ("ALT", None) => Ok(Command::Alt { secs: alt_default_s }),
        ("ALT", Some(n)) => match parse_u32(n) {
            Some(0) | None => Err(ParseError::BadArgument),
            Some(secs) => Ok(Command::Alt { secs }),
        },
        ("STOP", None) => Ok(Command::Stop),
        ("STATUS", None) => Ok(Command::Status),
        ("PAIR", Some("REMOTE")) => Ok(Command::PairRemote),
        ("FORGET", Some("REMOTE")) => Ok(Command::ForgetRemote),
        ("DRIVELOCK", Some("ON")) => Ok(Command::DriveLock(true)),
        ("DRIVELOCK", Some("OFF")) => Ok(Command::DriveLock(false)),
        ("STOP" | "STATUS" | "PAIR" | "FORGET" | "DRIVELOCK", _) => Err(ParseError::BadArgument),
        _ => Err(ParseError::Unknown),
    }
}

fn trim(input: &[u8]) -> &[u8] {
    let junk = |b: &u8| b.is_ascii_whitespace() || *b == 0;
    let start = input.iter().position(|b| !junk(b)).unwrap_or(input.len());
    let end = input.iter().rposition(|b| !junk(b)).map_or(start, |i| i + 1);
    &input[start..end]
}

fn parse_u32(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 10 {
        return None;
    }
    let mut v: u32 = 0;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Command, ParseError> {
        parse(s.as_bytes(), 8)
    }

    #[test]
    fn winks() {
        assert_eq!(p("WINK L"), Ok(Command::Wink(Side::Left)));
        assert_eq!(p("wink r"), Ok(Command::Wink(Side::Right)));
        assert_eq!(p("Wink B"), Ok(Command::Wink(Side::Both)));
        assert_eq!(p("WINK LEFT"), Ok(Command::Wink(Side::Left)));
        assert_eq!(p("WINK RH"), Ok(Command::Wink(Side::Right)));
        assert_eq!(p("WINK BOTH"), Ok(Command::Wink(Side::Both)));
        assert_eq!(p("WINK"), Err(ParseError::BadArgument));
        assert_eq!(p("WINK X"), Err(ParseError::BadArgument));
        assert_eq!(p("WINK L R"), Err(ParseError::BadArgument));
    }

    #[test]
    fn alt() {
        assert_eq!(p("ALT 8"), Ok(Command::Alt { secs: 8 }));
        assert_eq!(p("alt 3"), Ok(Command::Alt { secs: 3 }));
        assert_eq!(p("ALT"), Ok(Command::Alt { secs: 8 }));
        // Clamping is the controller's job; the parser keeps the request.
        assert_eq!(p("ALT 60"), Ok(Command::Alt { secs: 60 }));
        assert_eq!(p("ALT 0"), Err(ParseError::BadArgument));
        assert_eq!(p("ALT -1"), Err(ParseError::BadArgument));
        assert_eq!(p("ALT 1.5"), Err(ParseError::BadArgument));
        assert_eq!(p("ALT 99999999999"), Err(ParseError::BadArgument));
        assert_eq!(p("ALT 4294967295"), Ok(Command::Alt { secs: u32::MAX }));
        assert_eq!(p("ALT 4294967296"), Err(ParseError::BadArgument));
    }

    #[test]
    fn others() {
        assert_eq!(p("STOP"), Ok(Command::Stop));
        assert_eq!(p("stop"), Ok(Command::Stop));
        assert_eq!(p("STOP NOW"), Err(ParseError::BadArgument));
        assert_eq!(p("STATUS"), Ok(Command::Status));
        assert_eq!(p("PAIR REMOTE"), Ok(Command::PairRemote));
        assert_eq!(p("pair remote"), Ok(Command::PairRemote));
        assert_eq!(p("PAIR"), Err(ParseError::BadArgument));
        assert_eq!(p("PAIR PHONE"), Err(ParseError::BadArgument));
        assert_eq!(p("FORGET REMOTE"), Ok(Command::ForgetRemote));
        assert_eq!(p("DRIVELOCK ON"), Ok(Command::DriveLock(true)));
        assert_eq!(p("drivelock off"), Ok(Command::DriveLock(false)));
        assert_eq!(p("DRIVELOCK"), Err(ParseError::BadArgument));
        assert_eq!(p("DRIVELOCK MAYBE"), Err(ParseError::BadArgument));
        assert!(Command::Wink(Side::Left).moves_lamps());
        assert!(Command::Alt { secs: 8 }.moves_lamps());
        assert!(!Command::Stop.moves_lamps());
        assert!(!Command::DriveLock(true).moves_lamps());
        assert_eq!(p("HONK"), Err(ParseError::Unknown));
    }

    #[test]
    fn whitespace_and_junk() {
        assert_eq!(p("  WINK   L \r\n"), Ok(Command::Wink(Side::Left)));
        assert_eq!(parse(b"STOP\0\0", 8), Ok(Command::Stop));
        assert_eq!(parse(b"\tALT\t5\n", 8), Ok(Command::Alt { secs: 5 }));
        assert_eq!(p(""), Err(ParseError::Empty));
        assert_eq!(p("   \r\n"), Err(ParseError::Empty));
        assert_eq!(parse(&[0u8; 4], 8), Err(ParseError::Empty));
    }

    #[test]
    fn rejects_long_and_binary() {
        assert_eq!(p("WINK                                L"), Err(ParseError::TooLong));
        assert_eq!(parse(&[b'W', 0xC3, 0xA4], 8), Err(ParseError::NotAscii));
        assert_eq!(parse(&[0xFF; 200], 8), Err(ParseError::TooLong));
    }

    #[test]
    fn side_helpers() {
        assert!(Side::Left.lh() && !Side::Left.rh());
        assert!(!Side::Right.lh() && Side::Right.rh());
        assert!(Side::Both.lh() && Side::Both.rh());
    }
}
