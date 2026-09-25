//! Panel snapshot, presentation state, and window updates.
use gpui::{
    App, Bounds, Context, Entity, Pixels, RetainAllImageCache, Size, Window, point, px, size,
};
use mirix_core::{CandidateId, PanelSnapshot};
use mirix_ipc::Request;
use mirix_skin::Skin;
use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

use super::client::select_candidate;
use super::layout::{self, PanelLayout};
use super::motion::MotionState;
use super::popup::IDLE_REBUILD_AFTER;
use super::services::{PanelEvent, Services};

pub(super) struct CandidatePanel {
    pub(super) snapshot: PanelSnapshot,
    pub(super) show_preedit: bool,
    pub(super) skin: Skin,
    pub(super) skin_root: Option<PathBuf>,
    pub(super) skin_images: Option<Entity<RetainAllImageCache>>,
    pub(super) services: Services,
    pub(super) size: Size<Pixels>,
    pub(super) layout: PanelLayout,
    pub(super) motion: MotionState,
    pub(super) animations_enabled: bool,
    pub(super) quick_switch: Option<mirix_core::QuickSwitchSnapshot>,
    pub(super) picker_layout: super::quick_switch::PickerLayout,
    picker_dirty: bool,
    layout_dirty: bool,
    page_dirty: bool,
    available: Size<Pixels>,
    last_pages: Option<(u64, Vec<CandidateId>, Vec<usize>)>,
    panel_revision: Option<u64>,
    pending_selection: Option<(u64, CandidateId)>,
    selection_hides_panel: bool,
    mapped: bool,
    hidden_since: Option<SystemTime>,
    embedded: Option<Size<Pixels>>,
    popup_generation: u64,
    pub(super) show_started: Option<std::time::Instant>,
    pub(super) show_rebuild: Option<super::popup::ShowRebuild>,
}

impl CandidatePanel {
    /// An ordinary child view: no IPC, popup role, resize, mapping or input region changes.
    pub(super) fn preview(
        skin: Skin,
        root: Option<PathBuf>,
        images: Entity<RetainAllImageCache>,
    ) -> Self {
        let (services, _) = Services::start(None, false).expect("offline services");
        let available = size(px(720.), px(480.));
        let mut panel = Self::new(
            super::demo::snapshot(),
            skin,
            root,
            services,
            true,
            available,
        );
        panel.embedded = Some(available);
        panel.skin_images = Some(images);
        panel
    }

    pub(super) fn new(
        snapshot: PanelSnapshot,
        skin: Skin,
        skin_root: Option<PathBuf>,
        services: Services,
        show_preedit: bool,
        size: Size<Pixels>,
    ) -> Self {
        Self {
            snapshot,
            skin,
            skin_root,
            skin_images: None,
            services,
            show_preedit,
            size,
            layout: PanelLayout::default(),
            motion: MotionState::default(),
            animations_enabled: true,
            quick_switch: None,
            picker_layout: Default::default(),
            picker_dirty: false,
            layout_dirty: true,
            page_dirty: true,
            available: Size::default(),
            last_pages: None,
            panel_revision: None,
            pending_selection: None,
            selection_hides_panel: false,
            mapped: true,
            hidden_since: None,
            embedded: None,
            popup_generation: 0,
            show_started: None,
            show_rebuild: None,
        }
    }

    pub(super) fn visible(&self) -> bool {
        (self.pending_selection.is_none() || !self.selection_hides_panel)
            && snapshot_visible(
                &self.snapshot,
                self.snapshot
                    .show_panel_preedit(self.show_preedit && self.skin.layout.show_preedit),
            )
    }

    pub(super) fn confirm_selection(
        &mut self,
        context_id: u64,
        candidate_id: CandidateId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.begin_selection(context_id, candidate_id) {
            return;
        }
        if self.selection_hides_panel {
            self.hide(window);
        }
        if !select_candidate(self.services.commands(), context_id, candidate_id) {
            self.pending_selection = None;
        }
        self.update_presentation(window, cx);
    }

    fn begin_selection(&mut self, context_id: u64, candidate_id: CandidateId) -> bool {
        if self.pending_selection.is_some()
            || !self.visible()
            || self.quick_switch.is_some()
            || self.snapshot.context_id.map(|id| id.0) != Some(context_id)
            || !self
                .snapshot
                .candidates
                .iter()
                .any(|candidate| candidate.id == candidate_id)
        {
            return false;
        }
        self.selection_hides_panel = !self
            .snapshot
            .candidates
            .iter()
            .find(|c| c.id == candidate_id)
            .unwrap()
            .keep_open;
        self.pending_selection = Some((context_id, candidate_id));
        true
    }

    fn accept_snapshot(&mut self, revision: u64, mut snapshot: PanelSnapshot) -> bool {
        if self
            .panel_revision
            .is_some_and(|current| revision < current)
        {
            return false;
        }
        if snapshot.context_id != self.snapshot.context_id || !snapshot.visible {
            self.pending_selection = None;
            self.quick_switch = None;
            self.picker_dirty = true;
        }
        snapshot.selected = snapshot
            .selected
            .min(snapshot.candidates.len().saturating_sub(1) as u32);
        self.layout_dirty |= self.snapshot.panel_text() != snapshot.panel_text()
            || self.snapshot.show_panel_preedit(false) != snapshot.show_panel_preedit(false)
            || self.snapshot.preedit != snapshot.preedit
            || self.snapshot.candidates.len() != snapshot.candidates.len()
            || self
                .snapshot
                .candidates
                .iter()
                .zip(&snapshot.candidates)
                .any(|(previous, next)| {
                    // Labels are generated from our measured pages. Pagination
                    // acknowledgements and selection metadata do not change geometry.
                    previous.text != next.text
                        || previous.comment != next.comment
                        || previous.reading != next.reading
                });
        self.page_dirty |= self.snapshot.preedit_cursor != snapshot.preedit_cursor
            || self.snapshot.panel_composition != snapshot.panel_composition
            || self.snapshot.preedit_selection != snapshot.preedit_selection;
        self.page_dirty |= self.snapshot.selected != snapshot.selected
            || self.snapshot.context_id != snapshot.context_id
            || self.snapshot.visible != snapshot.visible
            // New identities need fresh pagination even when all text is unchanged.
            || !self.snapshot.candidates.iter().map(|candidate| candidate.id)
                .eq(snapshot.candidates.iter().map(|candidate| candidate.id));
        self.panel_revision = Some(revision);
        self.snapshot = snapshot;
        true
    }

    fn receive_event(&mut self, event: PanelEvent) -> bool {
        match event {
            PanelEvent::UiPreferenceChanged(_) => false,
            PanelEvent::InputMethod { .. }
            | PanelEvent::ApplySkin(_)
            | PanelEvent::ConfigurationFinished { .. }
            | PanelEvent::Catalog(_)
            | PanelEvent::CatalogFinished { .. } => false,
            PanelEvent::QuickSwitch { state } => {
                self.quick_switch = state;
                self.picker_dirty = true;
                true
            }
            PanelEvent::Snapshot { revision, snapshot } => self.accept_snapshot(revision, snapshot),
            PanelEvent::SelectionFinished {
                context_id,
                candidate_id,
                update,
            } => {
                if self.pending_selection != Some((context_id, candidate_id)) {
                    return false;
                }
                self.pending_selection = None;
                if let Some((revision, snapshot)) = update {
                    self.accept_snapshot(revision, snapshot);
                } else {
                    // Unknown outcome is not permission to resurrect the
                    // pre-click candidates. Wait for a fresh subscription update.
                    self.snapshot.visible = false;
                }
                true
            }
            PanelEvent::Disconnected => {
                self.quick_switch = None;
                self.picker_dirty = true;
                self.pending_selection = None;
                self.panel_revision = None;
                self.snapshot = empty_snapshot();
                self.layout_dirty = true;
                true
            }
        }
    }

    pub(super) fn preview_selection(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.snapshot.selected = index.min(self.snapshot.candidates.len().saturating_sub(1)) as u32;
        self.page_dirty = true;
        self.update_window_size(window, cx);
        cx.notify();
    }

    pub(super) fn set_animations_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.animations_enabled != enabled {
            self.animations_enabled = enabled;
            self.motion = MotionState::default();
            cx.notify();
        }
    }

    pub(super) fn set_skin(
        &mut self,
        skin: Skin,
        root: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.skin = skin;
        self.skin_root = root;
        // Keep images while hidden, but release the previous skin on replacement.
        self.skin_images = None;
        self.motion = MotionState::default();
        self.layout_dirty = true;
        self.last_pages = None;
        self.update_presentation(window, cx);
    }

    pub(super) fn apply_events(
        &mut self,
        events: impl IntoIterator<Item = PanelEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut changed = false;
        for event in events {
            if self.receive_event(event) {
                changed = true;
                // Preserve the hide boundary even if a new composition is
                // already queued. Only the final visible state is measured.
                if !self.visible() {
                    self.hide(window);
                }
            }
        }
        if !changed {
            return;
        }
        self.update_presentation(window, cx);
    }

    pub(super) fn language_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picker_dirty = true;
        self.update_presentation(window, cx);
    }

    pub(super) fn window_recreated(
        &mut self,
        generation: u64,
        hidden: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.popup_generation = generation;
        // A visible replacement starts mapped, so update_presentation will not
        // queue another rebuild. Hidden recovery replacements remain unmapped.
        self.mapped = !hidden;
        self.hidden_since = hidden.then(SystemTime::now);
        self.layout_dirty = true;
        self.picker_dirty = true;
        self.page_dirty = true;
        self.update_presentation(window, cx);
    }

    pub(super) fn hidden_for(&self, now: SystemTime) -> Option<Duration> {
        // Wall time includes suspend. A clock rollback conservatively expires
        // the window instead of allowing an unknown idle interval to persist.
        self.hidden_since
            .map(|since| now.duration_since(since).unwrap_or(IDLE_REBUILD_AFTER))
    }

    pub(super) fn can_rebuild_before_show(&self, now: SystemTime) -> bool {
        self.awaiting_native_show()
            && self
                .hidden_for(now)
                .is_some_and(|idle| idle >= IDLE_REBUILD_AFTER)
    }

    pub(super) fn awaiting_native_show(&self) -> bool {
        !self.mapped && self.visible() && self.embedded.is_none()
    }

    fn record_mapping(&mut self, mapped: bool, now: SystemTime) {
        if mapped {
            self.hidden_since = None;
        } else if self.mapped {
            self.hidden_since = Some(now);
        }
        self.mapped = mapped;
    }

    fn hide(&mut self, window: &Window) {
        self.show_started = None;
        if let Some(rebuild) = &self.show_rebuild {
            rebuild.cancel();
        }
        self.last_pages = None;
        // A remap must publish pagination again, even if the snapshot is unchanged.
        self.page_dirty = true;
        if self.mapped {
            match window.set_mapped(false) {
                Ok(()) => {
                    self.record_mapping(false, SystemTime::now());
                }
                Err(error) => log::error!("unmap: {error:#}"),
            }
            window.set_input_region(Some(&[]));
        }
        self.motion.sync(
            &self.snapshot,
            self.layout.page.start,
            false,
            std::time::Instant::now(),
        );
    }

    pub(super) fn update_presentation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.embedded.is_some() {
            self.update_window_size(window, cx);
            cx.notify();
            return;
        }
        if self.visible() {
            if self.can_rebuild_before_show(SystemTime::now())
                && let Some(rebuild) = &self.show_rebuild
            {
                // Keep the old window unmapped. The worker installs the latest
                // state in a new native window before its first visible frame.
                rebuild.request(std::time::Instant::now());
                return;
            }
            self.update_window_size(window, cx);
            if !self.mapped {
                self.update_input_region(window);
                // Mark the root dirty before the platform requests its first frame.
                window.refresh();
                match window.set_mapped(true) {
                    Ok(()) => self.record_mapping(true, SystemTime::now()),
                    Err(error) => log::error!("map: {error:#}"),
                }
            }
        } else {
            self.hide(window);
        }
        cx.notify();
    }

    pub(super) fn update_window_size(&mut self, window: &mut Window, cx: &App) {
        if let Some(available) = self.embedded {
            if self.layout_dirty || self.available != available {
                self.layout = layout::calculate(
                    &self.snapshot,
                    &self.skin,
                    self.show_preedit,
                    available,
                    window,
                );
                self.size = self.layout.size;
                self.available = available;
                self.layout_dirty = false;
            } else if self.page_dirty {
                self.layout.select(self.snapshot.selected as usize);
                self.size = self.layout.size;
            }
            self.page_dirty = false;
            return;
        }
        if !self.visible() {
            return;
        }
        let available = window
            .display(cx)
            .map(|display| display.visible_bounds().size)
            .map(|bounds| {
                size(
                    (bounds.width - px(32.0)).max(px(160.0)),
                    (bounds.height * 0.6).max(px(120.0)),
                )
            })
            .unwrap_or(size(px(1024.0), px(600.0)));
        if !self.layout_dirty
            && !self.page_dirty
            && !self.picker_dirty
            && self.available == available
        {
            // The first compositor configure may arrive after our initial
            // measurement and restore the startup size. Reconcile the surface
            // with the measured bounds instead of keeping a clipped buffer.
            if window.viewport_size() != self.size {
                window.resize(self.size);
            }
            return;
        }
        if self.layout_dirty || self.available != available {
            self.layout = layout::calculate(
                &self.snapshot,
                &self.skin,
                self.show_preedit,
                available,
                window,
            );
        } else if self.page_dirty {
            self.layout.select(self.snapshot.selected as usize);
        }
        self.available = available;
        self.layout_dirty = false;
        self.page_dirty = false;
        self.picker_dirty = false;
        self.picker_layout = super::quick_switch::calculate(
            self.quick_switch.as_ref(),
            available,
            self.layout.size,
            window,
            &cx.global::<crate::i18n::UiLanguage>().localizer,
        );
        let next = size(
            self.layout.size.width.max(self.picker_layout.size.width),
            self.layout.size.height + self.picker_offset(),
        );
        if next != self.size || (window.viewport_size() != next) {
            self.size = next;
            window.resize(next);
        }
        self.update_input_region(window);
        if let Some(context) = self.snapshot.context_id {
            let ids = self
                .snapshot
                .candidates
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let pages = (context.0, ids, self.layout.ends.clone());
            if self.last_pages.as_ref() != Some(&pages) {
                if let Some(commands) = self.services.commands() {
                    let _ = commands.send(Request::SetCandidatePages {
                        id: pages.0,
                        candidate_ids: pages.1.clone(),
                        ends: pages.2.clone(),
                    });
                }
                self.last_pages = Some(pages);
            }
        } else {
            self.last_pages = None;
        }
    }

    pub(super) fn body_bounds(&self) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(0.0), self.picker_offset()),
            size: self.layout.size,
        }
    }

    pub(super) fn report_rebuilt_show(&mut self, window: &Window) {
        if let Some(started) = self.show_started.take() {
            log::info!(target: "mirix_panel::popup",
                "native popup rebuilt show first render generation={} show_to_render_us={} context={:?} viewport={:?} scale={} (not presentation)",
                self.popup_generation, started.elapsed().as_micros(), self.snapshot.context_id,
                window.viewport_size(), window.scale_factor());
        }
    }

    fn picker_offset(&self) -> Pixels {
        if self.quick_switch.is_some() {
            self.picker_layout.size.height + px(super::quick_switch::GAP)
        } else {
            px(0.0)
        }
    }

    fn update_input_region(&self, window: &Window) {
        if self.visible() {
            let mut regions = vec![self.body_bounds()];
            if self.quick_switch.is_some() {
                regions.push(Bounds {
                    origin: point(px(0.0), px(0.0)),
                    size: self.picker_layout.size,
                });
            }
            window.set_input_region(Some(&regions));
        } else {
            window.set_input_region(Some(&[]));
        }
    }
}

pub(super) fn empty_snapshot() -> PanelSnapshot {
    PanelSnapshot {
        punctuation_mode: None,
        context_id: None,
        preedit: String::new(),
        panel_composition: mirix_core::PanelPreedit::default(),
        preedit_cursor: 0,
        preedit_selection: None,
        candidates: Vec::new(),
        selected: 0,
        visible: false,
    }
}

fn snapshot_visible(snapshot: &PanelSnapshot, show_preedit: bool) -> bool {
    snapshot.visible
        && (!snapshot.candidates.is_empty() || (show_preedit && !snapshot.panel_text().is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_rebuild_requires_visible_content_in_an_unmapped_native_panel() {
        let mut panel = composing_panel();
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let expired = start + Duration::from_secs(60);
        assert!(!panel.can_rebuild_before_show(expired));
        panel.record_mapping(false, start);
        assert!(!panel.can_rebuild_before_show(expired - Duration::from_millis(1)));
        assert!(panel.can_rebuild_before_show(expired));
        panel.snapshot.visible = false;
        assert!(!panel.can_rebuild_before_show(expired));
        panel.snapshot.visible = true;
        panel.embedded = Some(size(px(720.), px(480.)));
        assert!(!panel.can_rebuild_before_show(expired));
        panel.embedded = None;
        panel.record_mapping(true, expired);
        assert!(!panel.can_rebuild_before_show(expired));
    }

    #[test]
    fn hidden_updates_preserve_idle_time_but_showing_starts_a_new_idle_period() {
        let mut panel = composing_panel();
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        panel.record_mapping(false, start);
        panel.record_mapping(false, start + IDLE_REBUILD_AFTER / 2);
        assert!(panel.can_rebuild_before_show(start + IDLE_REBUILD_AFTER));

        let shown = start + IDLE_REBUILD_AFTER;
        panel.record_mapping(true, shown);
        assert_eq!(panel.hidden_for(shown), None);
        assert!(!panel.can_rebuild_before_show(shown + IDLE_REBUILD_AFTER));
        let hidden_again = shown + Duration::from_secs(1);
        panel.record_mapping(false, hidden_again);
        assert!(
            !panel.can_rebuild_before_show(
                hidden_again + IDLE_REBUILD_AFTER - Duration::from_secs(1)
            )
        );
        assert!(panel.can_rebuild_before_show(hidden_again + IDLE_REBUILD_AFTER));
    }

    #[test]
    fn clock_rollback_expires_a_hidden_window_without_panicking() {
        let mut panel = composing_panel();
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        panel.record_mapping(false, start);
        assert!(panel.can_rebuild_before_show(start - Duration::from_secs(1)));
    }

    fn composing_panel() -> CandidatePanel {
        let (services, _) = Services::start(None, true).unwrap();
        let mut snapshot = super::super::demo::snapshot();
        snapshot.context_id = Some(mirix_core::ContextId(1));
        let mut panel = CandidatePanel::new(
            snapshot.clone(),
            Skin::default(),
            None,
            services,
            true,
            size(px(600.0), px(64.0)),
        );
        panel.accept_snapshot(10, snapshot);
        panel
    }

    #[test]
    fn pagination_labels_and_selection_metadata_reuse_candidate_measurements() {
        let mut panel = composing_panel();
        panel.layout_dirty = false;
        panel.page_dirty = false;
        let mut next = panel.snapshot.clone();
        for (index, candidate) in next.candidates.iter_mut().enumerate() {
            candidate.label = (index % 3 + 1).to_string();
        }
        assert!(panel.accept_snapshot(11, next.clone()));
        assert!(!panel.layout_dirty);
        assert!(!panel.page_dirty);
        assert_eq!(panel.snapshot, next);

        next.candidates[3].id = CandidateId(100);
        next.candidates[3].keep_open = true;
        assert!(panel.accept_snapshot(12, next.clone()));
        assert!(!panel.layout_dirty);
        assert!(panel.page_dirty);

        panel.page_dirty = false;
        next.selected = 3;
        assert!(panel.accept_snapshot(13, next));
        assert!(!panel.layout_dirty);
        assert!(panel.page_dirty);
        assert!(!panel.begin_selection(1, CandidateId(4)));
        assert!(panel.begin_selection(1, CandidateId(100)));
        assert!(panel.visible());
    }

    #[test]
    fn candidate_content_order_and_count_changes_invalidate_measurements() {
        let changes: [fn(&mut PanelSnapshot); 6] = [
            |snapshot| snapshot.candidates[0].text.push('字'),
            |snapshot| snapshot.candidates[0].reading = Some("zhōng wén".into()),
            |snapshot| snapshot.candidates[0].comment = Some("description".into()),
            |snapshot| snapshot.candidates.swap(0, 1),
            |snapshot| {
                snapshot.candidates.pop();
            },
            |snapshot| snapshot.candidates.push(snapshot.candidates[0].clone()),
        ];
        for change in changes {
            let mut panel = composing_panel();
            panel.layout_dirty = false;
            let mut next = panel.snapshot.clone();
            change(&mut next);
            assert!(panel.accept_snapshot(11, next));
            assert!(panel.layout_dirty);
        }
    }

    #[test]
    fn selection_hides_before_reply_and_old_updates_cannot_reopen_it() {
        let mut panel = composing_panel();
        let composing = panel.snapshot.clone();
        assert!(panel.begin_selection(1, CandidateId(1)));
        assert!(!panel.visible());
        assert!(!panel.begin_selection(1, CandidateId(1))); // no double submission
        assert!(panel.accept_snapshot(11, composing.clone())); // already in flight
        assert!(!panel.visible());

        let mut committed = empty_snapshot();
        committed.context_id = composing.context_id;
        panel.receive_event(PanelEvent::SelectionFinished {
            context_id: 1,
            candidate_id: CandidateId(1),
            update: Some((12, committed)),
        });
        assert!(!panel.visible());
        assert!(!panel.accept_snapshot(11, composing.clone()));
        assert!(!panel.visible());
        assert!(panel.accept_snapshot(13, composing)); // a genuinely new composition
        assert!(panel.visible());
    }

    #[test]
    fn keep_open_selection_stays_visible_but_rejects_duplicate_clicks() {
        let mut panel = composing_panel();
        panel.snapshot.candidates[0].keep_open = true;
        assert!(panel.begin_selection(1, CandidateId(1)));
        assert!(panel.visible());
        assert!(!panel.begin_selection(1, CandidateId(1)));
        let mut next = panel.snapshot.clone();
        next.preedit = "今日は良い天気です".into();
        next.preedit_selection = Some(9..15);
        next.preedit_cursor = 15;
        assert!(panel.accept_snapshot(11, next.clone()));
        assert!(panel.visible());
        panel.receive_event(PanelEvent::SelectionFinished {
            context_id: 1,
            candidate_id: CandidateId(1),
            update: Some((12, next)),
        });
        assert!(panel.visible());
        assert!(panel.pending_selection.is_none());
        assert_eq!(panel.snapshot.preedit_selection, Some(9..15));
        panel.receive_event(PanelEvent::Disconnected);
        assert!(!panel.visible());
    }

    #[test]
    fn selection_reply_does_not_replace_newer_input() {
        for update in [Some((12, empty_snapshot())), Some((10, empty_snapshot()))] {
            let mut panel = composing_panel();
            assert!(panel.begin_selection(1, CandidateId(1)));
            let mut newer = panel.snapshot.clone();
            newer.preedit = "new input".into();
            panel.accept_snapshot(13, newer.clone());
            panel.receive_event(PanelEvent::SelectionFinished {
                context_id: 1,
                candidate_id: CandidateId(1),
                update,
            });
            assert!(panel.visible());
            assert_eq!(panel.snapshot, newer);
        }
    }

    #[test]
    fn unknown_selection_outcome_stays_hidden_until_fresh_state() {
        let mut panel = composing_panel();
        let composing = panel.snapshot.clone();
        assert!(panel.begin_selection(1, CandidateId(1)));
        panel.receive_event(PanelEvent::SelectionFinished {
            context_id: 1,
            candidate_id: CandidateId(1),
            update: None,
        });
        assert!(!panel.visible());
        panel.accept_snapshot(11, composing);
        assert!(panel.visible());
    }

    #[test]
    fn authoritative_hide_releases_selection_without_waiting_for_command_reply() {
        let mut panel = composing_panel();
        let composing = panel.snapshot.clone();
        assert!(panel.begin_selection(1, CandidateId(1)));
        let mut hidden = empty_snapshot();
        hidden.context_id = composing.context_id;
        panel.accept_snapshot(11, hidden);
        assert!(!panel.visible());
        panel.accept_snapshot(12, composing.clone());
        assert!(panel.visible());
        assert!(!panel.receive_event(PanelEvent::SelectionFinished {
            context_id: 1,
            candidate_id: CandidateId(1),
            update: None,
        }));
        assert_eq!(panel.snapshot, composing);
    }

    #[test]
    fn context_change_or_disconnect_cancels_pending_selection() {
        let mut panel = composing_panel();
        assert!(panel.begin_selection(1, CandidateId(1)));
        let mut other = panel.snapshot.clone();
        other.context_id = Some(mirix_core::ContextId(2));
        panel.accept_snapshot(11, other.clone());
        assert!(panel.visible());
        assert!(!panel.receive_event(PanelEvent::SelectionFinished {
            context_id: 1,
            candidate_id: CandidateId(1),
            update: Some((12, empty_snapshot())),
        }));
        assert_eq!(panel.snapshot, other);
        assert!(panel.begin_selection(2, CandidateId(1)));
        panel.receive_event(PanelEvent::Disconnected);
        assert!(!panel.visible());
        assert!(!panel.receive_event(PanelEvent::SelectionFinished {
            context_id: 2,
            candidate_id: CandidateId(1),
            update: Some((12, other.clone())),
        }));
        assert!(panel.accept_snapshot(0, other)); // fresh daemon starts a new revision stream
        assert!(panel.visible());
    }

    #[test]
    fn candidate_body_starts_at_window_origin_without_external_space() {
        let mut panel = composing_panel();
        for (width, height) in [(500.0, 64.0), (80.0, 200.0), (760.0, 420.0)] {
            let body = size(px(width), px(height));
            panel.layout.size = body;
            let input = panel.body_bounds();
            assert_eq!(input.origin, point(px(0.0), px(0.0)));
            assert_eq!(input.size, body);
        }
    }

    #[test]
    fn preedit_only_snapshot_is_visible_only_when_preedit_is_drawn() {
        let mut snapshot = empty_snapshot();
        snapshot.visible = true;
        snapshot.preedit = "ni".into();
        assert!(snapshot_visible(&snapshot, true));
        assert!(!snapshot_visible(&snapshot, false));
        snapshot.visible = false;
        assert!(!snapshot_visible(&snapshot, true));
    }

    #[test]
    fn engine_header_overrides_hidden_preedit_and_marks_layout_dirty() {
        let mut panel = composing_panel();
        panel.show_preedit = false;
        panel.skin.layout.show_preedit = false;
        panel.snapshot.candidates.clear();
        panel.snapshot.preedit = "kimi".into();
        assert!(!panel.visible());
        let mut next = panel.snapshot.clone();
        next.panel_composition = mirix_core::PanelPreedit::Composition(mirix_core::Composition {
            text: "きみ".into(),
            cursor: 6,
            selection: None,
        });
        panel.layout_dirty = false;
        assert!(panel.accept_snapshot(11, next.clone()));
        assert!(panel.visible() && panel.layout_dirty);
        panel.layout_dirty = false;
        if let mirix_core::PanelPreedit::Composition(panel) = &mut next.panel_composition {
            panel.text = "君".into();
        }
        assert!(panel.accept_snapshot(12, next.clone()));
        assert!(panel.layout_dirty);
        panel.show_preedit = true;
        panel.skin.layout.show_preedit = true;
        next.panel_composition = mirix_core::PanelPreedit::Hidden;
        assert!(panel.accept_snapshot(13, next));
        assert!(!panel.visible());
        panel.receive_event(PanelEvent::Disconnected);
        assert!(!panel.visible());
        assert_eq!(
            panel.snapshot.panel_composition,
            mirix_core::PanelPreedit::FollowApplication
        );
    }

    #[test]
    fn picker_does_not_replace_candidate_state_and_focus_change_clears_it() {
        let mut panel = composing_panel();
        panel.layout.size = size(px(620.0), px(72.0));
        panel.layout.page = 0..3;
        panel.layout.ends = vec![3, 5];
        panel.layout_dirty = false;
        let before = panel.snapshot.clone();
        panel.receive_event(PanelEvent::QuickSwitch {
            state: Some(mirix_core::QuickSwitchSnapshot {
                label: String::new(),
                session: 1,
                kind: mirix_core::QuickSwitchKind::Skin,
                entries: vec![mirix_core::MenuEntry {
                    id: "night".into(),
                    label: "night".into(),
                }],
                selected: 0,
                active: Some("night".into()),
                error: None,
            }),
        });
        panel.picker_layout.size = size(px(240.0), px(94.0));
        assert_eq!(panel.snapshot, before);
        assert_eq!(panel.layout.page, 0..3);
        assert_eq!(panel.layout.ends, vec![3, 5]);
        assert!(!panel.layout_dirty);
        assert_eq!(panel.body_bounds().size, size(px(620.0), px(72.0)));
        assert_eq!(panel.body_bounds().origin, point(px(0.0), px(100.0)));
        assert!(!panel.begin_selection(1, CandidateId(1)));
        let mut next = before;
        next.context_id = Some(mirix_core::ContextId(2));
        panel.accept_snapshot(11, next);
        assert!(panel.quick_switch.is_none());
        assert_eq!(panel.body_bounds().origin, point(px(0.0), px(0.0)));
    }
}
