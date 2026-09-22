//! Nonblocking framed output. A partially written frame must always be completed.
use std::collections::VecDeque;
use std::io::{self, Write};

#[derive(Default)]
pub(super) struct Outbound {
    frames: VecDeque<(Vec<u8>, bool)>,
    offset: usize,
}

impl Outbound {
    pub fn push(&mut self, frame: Vec<u8>, snapshot: bool) {
        // Only replace a wholly unsent trailing snapshot. Replies and the
        // bytes of an in-flight frame retain their order and framing.
        if snapshot
            && self.frames.back().is_some_and(|(_, snapshot)| *snapshot)
            && (self.frames.len() > 1 || self.offset == 0)
        {
            self.frames.pop_back();
        }
        self.frames.push_back((frame, snapshot));
    }

    /// Returns true once drained; false requests a writable notification.
    pub fn flush(&mut self, writer: &mut impl Write) -> io::Result<bool> {
        while let Some((frame, _)) = self.frames.front() {
            match writer.write(&frame[self.offset..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(written) => {
                    self.offset += written;
                    if self.offset == frame.len() {
                        self.frames.pop_front();
                        self.offset = 0;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) => return Err(error),
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirix_core::PanelSnapshot;
    use mirix_ipc::{Response, decode_frame, encode_frame};

    #[derive(Default)]
    struct Writer {
        budget: usize,
        bytes: Vec<u8>,
    }

    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.budget == 0 {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let count = bytes.len().min(self.budget);
            self.bytes.extend_from_slice(&bytes[..count]);
            self.budget -= count;
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn snapshot(revision: u64, visible: bool) -> Vec<u8> {
        encode_frame(&Response::panel_update(
            &PanelSnapshot {
                preedit_cursor: 0,
                preedit_selection: None,
                panel_composition: mirix_core::PanelPreedit::default(),
                punctuation_mode: None,
                context_id: None,
                preedit: "ni".into(),
                candidates: vec![],
                selected: 0,
                visible,
            },
            revision,
        ))
        .unwrap()
    }

    #[test]
    fn partial_frame_finishes_before_latest_hide_and_reply() {
        for budget in [1, 3, 8, 32] {
            let first = snapshot(1, true);
            let hidden = snapshot(3, false);
            let reply = encode_frame(&Response::Ok).unwrap();
            let mut queue = Outbound::default();
            let mut writer = Writer {
                budget,
                ..Default::default()
            };
            queue.push(first.clone(), true);
            assert!(!queue.flush(&mut writer).unwrap());
            queue.push(snapshot(2, true), true);
            queue.push(hidden.clone(), true);
            queue.push(reply.clone(), false);
            assert!(!queue.flush(&mut writer).unwrap());
            writer.budget = usize::MAX;
            assert!(queue.flush(&mut writer).unwrap());
            assert_eq!(writer.bytes, [first, hidden, reply].concat());
            let mut bytes = writer.bytes.as_slice();
            for _ in 0..3 {
                let (_, used) = decode_frame::<Response>(bytes).unwrap().unwrap();
                bytes = &bytes[used..];
            }
            assert!(bytes.is_empty());
        }
    }

    #[test]
    fn backpressure_keeps_only_latest_unsent_snapshot_without_losing_replies() {
        let mut queue = Outbound::default();
        let reply = encode_frame(&Response::Ok).unwrap();
        queue.push(reply.clone(), false);
        for revision in 0..1000 {
            queue.push(snapshot(revision, true), true);
        }
        let hidden = snapshot(1000, false);
        queue.push(hidden.clone(), true);
        assert_eq!(queue.frames.len(), 2);
        let mut writer = Writer {
            budget: usize::MAX,
            ..Default::default()
        };
        assert!(queue.flush(&mut writer).unwrap());
        assert_eq!(writer.bytes, [reply, hidden].concat());
    }
}
