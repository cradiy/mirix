//! A single deadline covers both directions of a nonblocking protocol exchange.
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::time::Instant;

use anyhow::{Result, bail};
use mirix_engine_protocol::{EngineResponse, decode_frame};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};

pub(super) fn nonblocking(fd: &impl AsFd) -> Result<()> {
    fcntl_setfl(fd, fcntl_getfl(fd)? | OFlags::NONBLOCK)?;
    Ok(())
}

fn ready(fd: &impl AsFd, interest: PollFlags, deadline: Instant) -> Result<()> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("input method exchange timed out");
        }
        let timeout = Timespec {
            tv_sec: remaining.as_secs().try_into().unwrap_or(i64::MAX),
            tv_nsec: remaining.subsec_nanos().into(),
        };
        let mut fds = [PollFd::new(fd, interest)];
        match poll(&mut fds, Some(&timeout)) {
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
            Ok(0) => bail!("input method exchange timed out"),
            Ok(_) => {}
        }
        let events = fds[0].revents();
        if events.contains(interest) {
            return Ok(());
        }
        if events.intersects(PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL) {
            bail!("input method protocol stream closed");
        }
    }
}

pub(super) fn write_frame(
    writer: &mut (impl Write + AsFd),
    mut frame: &[u8],
    deadline: Instant,
) -> Result<()> {
    while !frame.is_empty() {
        ready(writer, PollFlags::OUT, deadline)?;
        match writer.write(frame) {
            Ok(0) => bail!("input method protocol stream closed"),
            Ok(count) => frame = &frame[count..],
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(super) fn read_response(
    reader: &mut (impl Read + AsFd),
    buffer: &mut Vec<u8>,
    deadline: Instant,
) -> Result<EngineResponse> {
    loop {
        if let Some((response, used)) = decode_frame::<EngineResponse>(buffer)? {
            buffer.drain(..used);
            return Ok(response);
        }
        ready(reader, PollFlags::IN, deadline)?;
        let mut chunk = [0_u8; 4096];
        match reader.read(&mut chunk) {
            Ok(0) => bail!("input method protocol stream closed"),
            Ok(count) => buffer.extend_from_slice(&chunk[..count]),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::net::UnixStream, time::Duration};

    #[test]
    fn an_unresponsive_reader_cannot_block_writes_indefinitely() {
        let (mut writer, _reader) = UnixStream::pair().unwrap();
        nonblocking(&writer).unwrap();
        let started = Instant::now();
        let error = write_frame(
            &mut writer,
            &vec![0; 4 * 1024 * 1024],
            started + Duration::from_millis(30),
        )
        .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn closed_and_silent_response_streams_report_errors() {
        let (mut reader, peer) = UnixStream::pair().unwrap();
        nonblocking(&reader).unwrap();
        let error = read_response(
            &mut reader,
            &mut vec![],
            Instant::now() + Duration::from_millis(20),
        )
        .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        drop(peer);
        let error = read_response(
            &mut reader,
            &mut vec![],
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(error.to_string().contains("closed"));
    }
}
