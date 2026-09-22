//! Coalesce ordinary storage events without delaying explicit flush barriers.
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

use super::WriteEvent;

const BATCH_WINDOW: Duration = Duration::from_millis(25);
const MAX_EVENTS: usize = 256;

pub(super) fn receive(receiver: &mpsc::Receiver<WriteEvent>) -> (Vec<WriteEvent>, bool) {
    let first = match receiver.recv_timeout(Duration::from_secs(2)) {
        Ok(event) => event,
        Err(mpsc::RecvTimeoutError::Timeout) => return (Vec::new(), false),
        Err(mpsc::RecvTimeoutError::Disconnected) => return (Vec::new(), true),
    };
    let deadline = Instant::now() + BATCH_WINDOW;
    let mut events = vec![first];
    while events.len() < MAX_EVENTS {
        if matches!(
            events.last(),
            Some(WriteEvent::Checkpoint(_) | WriteEvent::Manage { .. } | WriteEvent::Retry)
        ) {
            break;
        }
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        match receiver.recv_timeout(deadline - now) {
            Ok(event) => events.push(event),
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => return (events, true),
        }
    }
    (events, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_ends_the_batch_before_later_changes() {
        let (sender, receiver) = mpsc::channel();
        for index in 0..20 {
            sender
                .send(WriteEvent::Activate(index.to_string()))
                .unwrap();
        }
        let (reply, _) = mpsc::channel();
        sender.send(WriteEvent::Checkpoint(reply)).unwrap();
        sender.send(WriteEvent::Activate("after".into())).unwrap();
        drop(sender);
        let (events, closed) = receive(&receiver);
        assert!(!closed);
        assert_eq!(events.len(), 21);
        assert!(matches!(events.last(), Some(WriteEvent::Checkpoint(_))));
        let (events, closed) = receive(&receiver);
        assert!(closed);
        assert!(matches!(events.as_slice(), [WriteEvent::Activate(name)] if name == "after"));
    }
}
