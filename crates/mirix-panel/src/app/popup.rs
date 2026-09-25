//! Native-window replacement without restarting the daemon or engine.
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context as _, Result};
use gpui::{App, AppContext, Context, Entity, Window, WindowHandle};

use super::{panel_window_options, view::CandidatePanel};

// Workaround: after prolonged inactivity (including suspend), the native popup
// has exhibited both slow animation and incorrect placement. Recreating the
// window restored both, but did not establish whether GPUI or the compositor
// caused the problem. Keep this until the underlying cause is verified fixed.
// Ten minutes is a chosen idle threshold, not a measured failure boundary.
// Rebuild before the next show to avoid a visible replacement/flicker; reuse the
// window during active input. This preserves the panel, engine and connection,
// and does not periodically create windows while the input method is idle.
pub(super) const IDLE_REBUILD_AFTER: Duration = Duration::from_secs(10 * 60);

/// Coalesce updates while waiting to replace a hidden window before mapping it.
#[derive(Clone)]
pub(super) struct ShowRebuild {
    requested_at: Rc<Cell<Option<Instant>>>,
    wake: async_channel::Sender<()>,
}

impl ShowRebuild {
    fn new() -> (Self, async_channel::Receiver<()>) {
        let (wake, requests) = async_channel::bounded(1);
        (
            Self {
                requested_at: Default::default(),
                wake,
            },
            requests,
        )
    }

    pub fn request(&self, now: Instant) {
        if self.requested_at.get().is_some() {
            return;
        }
        self.requested_at.set(Some(now));
        let _ = self.wake.try_send(());
    }

    pub fn cancel(&self) {
        self.requested_at.set(None);
    }

    fn take(&self) -> Option<Instant> {
        self.requested_at.take()
    }
}

/// All GUI consumers resolve the current handle at use time, including async
/// skin loads that began before a rebuild.
#[derive(Clone)]
pub(super) struct PanelWindow {
    handle: Rc<Cell<Option<WindowHandle<CandidatePanel>>>>,
    ready: async_channel::Receiver<()>,
    wake: async_channel::Sender<()>,
}

impl PanelWindow {
    pub fn new(handle: WindowHandle<CandidatePanel>) -> Self {
        let (wake, ready) = async_channel::bounded(1);
        Self {
            handle: Rc::new(Cell::new(Some(handle))),
            ready,
            wake,
        }
    }

    pub fn is_available(&self) -> bool {
        self.handle.get().is_some()
    }

    pub async fn wait_ready(&self) {
        while !self.is_available() {
            if self.ready.recv().await.is_err() {
                break;
            }
        }
    }

    pub fn update<C: AppContext, R>(
        &self,
        cx: &mut C,
        update: impl FnOnce(&mut CandidatePanel, &mut Window, &mut Context<CandidatePanel>) -> R,
    ) -> Result<R> {
        self.handle
            .get()
            .context("candidate window unavailable; restart Mirix")?
            .update(cx, update)
    }
}

pub(super) fn install(window: PanelWindow, cx: &mut App) {
    let (show, show_requests) = ShowRebuild::new();
    let Ok(panel) = window.update(cx, |panel, _, cx| {
        panel.show_rebuild = Some(show.clone());
        cx.entity()
    }) else {
        return;
    };
    let generation = Rc::new(Cell::new(0_u64));
    install_show_rebuild(window, panel, generation, show, show_requests, cx);
}

fn rebuild(
    window: &PanelWindow,
    panel: &Entity<CandidatePanel>,
    generation: &Cell<u64>,
    reason: &str,
    cx: &mut App,
) -> Result<()> {
    let next = generation.get() + 1;
    generation.set(next);
    let generation = next;
    let started = Instant::now();
    panel.update(cx, |panel, _| {
        panel.show_started = panel.show_rebuild.as_ref().and_then(ShowRebuild::take);
    });
    if let Some(old) = window.handle.get() {
        old.update(cx, |_, old_window, _| {
            old_window.remove_window();
        })?;
        window.handle.set(None);
    }

    let size = panel.read(cx).size;
    let hidden = !panel.read(cx).visible();
    let mut mapping_error = None;
    let replacement = cx.open_window(panel_window_options(size, true), |new_window, cx| {
        new_window.set_input_region(Some(&[]));
        // Recovery may happen after composition ended.
        // Keep that replacement hidden before its first root render.
        if hidden {
            mapping_error = new_window.set_mapped(false).err();
        }
        panel.update(cx, |panel, cx| {
            panel.window_recreated(generation, hidden, new_window, cx);
        });
        panel.clone()
    })?;
    if let Some(error) = mapping_error {
        replacement.update(cx, |_, window, _| window.remove_window())?;
        return Err(error.context("keep replacement popup hidden"));
    }
    window.handle.set(Some(replacement));
    let _ = window.wake.try_send(());
    log::info!(target: "mirix_panel::popup",
        "native popup rebuild complete reason={reason} hidden={hidden} generation={} new_window={:?} elapsed_ms={} engine_restart=false connection_replaced=false",
        generation, replacement, started.elapsed().as_millis());
    Ok(())
}

fn install_show_rebuild(
    window: PanelWindow,
    panel: Entity<CandidatePanel>,
    generation: Rc<Cell<u64>>,
    show: ShowRebuild,
    requests: async_channel::Receiver<()>,
    cx: &App,
) {
    log::info!(target: "mirix_panel::popup",
        "native popup idle rebuild enabled idle_threshold_secs={} hidden_preparation=false pid={}",
        IDLE_REBUILD_AFTER.as_secs(), std::process::id());
    cx.spawn(async move |cx| {
        while requests.recv().await.is_ok() {
            let result = cx.update(|cx| -> Result<()> {
                // The latest state wins if input was cleared before this task ran.
                let now = SystemTime::now();
                // Expiry was checked when scheduling. Do not undo the decision
                // if the wall clock is adjusted while this request is queued.
                if !panel.read(cx).awaiting_native_show() {
                    show.cancel();
                    return Ok(());
                }
                let Some(requested) = show.requested_at.get() else { return Ok(()); };
                log::info!(target: "mirix_panel::popup",
                    "native popup show rebuild requested idle_ms={} queue_us={}",
                    panel.read(cx).hidden_for(now).unwrap_or_default().as_millis(),
                    requested.elapsed().as_micros());
                rebuild(&window, &panel, &generation, "idle_before_show", cx)
            });
            if let Err(error) = result {
                log::error!(target: "mirix_panel::popup",
                    "popup show rebuild failed error={error:#}; input state retained");
                // If creation failed after retiring the old window, allow two
                // delayed recovery tries without an unbounded idle rebuild loop.
                for attempt in 1..=2 {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if window.is_available() { break; }
                    let result = cx.update(|cx| rebuild(&window, &panel, &generation, "before_show_retry", cx));
                    if result.is_ok() { break; }
                    log::error!(target: "mirix_panel::popup",
                        "popup show rebuild retry failed attempt={} error={:#}; restart Mirix if the candidate window remains unavailable",
                        attempt, result.unwrap_err());
                }
            }
        }
    }).detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hiding_cancels_a_queued_show_and_next_show_uses_its_own_timestamp() {
        let (show, requests) = ShowRebuild::new();
        let start = Instant::now();
        show.request(start);
        show.cancel();
        assert_eq!(show.take(), None);
        let again = start + Duration::from_millis(100);
        show.request(again);
        // The stale wake also services the new request without losing it.
        assert!(requests.try_recv().is_ok());
        assert_eq!(show.take(), Some(again));
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn repeated_updates_preserve_first_show_time_and_request_one_rebuild() {
        let (show, requests) = ShowRebuild::new();
        let start = Instant::now();
        for offset in 0..20 {
            show.request(start + Duration::from_millis(offset));
        }
        assert!(requests.try_recv().is_ok());
        assert!(requests.try_recv().is_err());
        assert_eq!(show.take(), Some(start));
        assert_eq!(show.take(), None);
        assert!(requests.try_recv().is_err());
    }
}
