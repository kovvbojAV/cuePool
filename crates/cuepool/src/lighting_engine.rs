//! Lighting cue playback — crossfades fixture looks, plays recorded DMX
//! shows, and streams the merged result.
//!
//! Each fixture fades independently: `go()` starts from its *current* look
//! (mid-fade included). Fixtures absent from a cue's snapshot keep their
//! existing fade or held look (LTP). Recorded `.dmxrec` shows ([`ShowPlayer`])
//! run alongside as separate merge layers; per tick every layer is composited
//! sACN-style (highest priority wins per owned channel, weight = fade envelope). A paced
//! [`DmxSender`] thread handles wire pacing/keep-alive; this engine only
//! submits frames when something changed.

use std::collections::BTreeMap;
use std::time::Instant;

use cuepool_core::lighting::{
    FixtureId, FixtureLook, LightingConfig, LightingProtocol, render_look,
};
use cuepool_core::{FadeType, LoopMode};
use rust_decimal::Decimal;
use rustjay_lighting::{
    ArtNetTransport, Dest, DmxSender, DmxTransport, MaskedFrame, RecEvent, SacnTransport,
    ShowPlayer, color_pipeline, composite, demux_tile,
};

struct ActiveFade {
    start: Instant,
    duration: f32,
    fade_type: FadeType,
    from: FixtureLook,
    to: FixtureLook,
}

/// A playing recorded DMX show (one `DmxShowCue`).
struct ActiveShow {
    qid: Decimal,
    player: ShowPlayer,
    started: Instant,
    priority: u8,
    loop_mode: LoopMode,
    loop_count: i32,
    fade_in: f32,
    /// Tail fade starting `fade_out` seconds before the natural end (mirrors
    /// SoundCue). Ignored for HoldLast and infinite loops.
    fade_out: f32,
    fade_type: FadeType,
    /// Stop-cue fade-out: (start, duration, curve).
    stopping: Option<(Instant, f32, FadeType)>,
    /// Natural end already announced (AfterLast chains fire once).
    finished_reported: bool,
}

impl ActiveShow {
    /// Advance the playhead to `now` and compute the merge weight.
    /// `None` = fully released — remove the show. Pushes the qid to
    /// `finished` the first time the show ends (naturally or by stop).
    fn eval(&mut self, now: Instant, finished: &mut Vec<Decimal>) -> Option<f32> {
        let dur_ms = self.player.duration_ms() as u64;
        if dur_ms == 0 {
            if !self.finished_reported {
                self.finished_reported = true;
                finished.push(self.qid);
            }
            return None;
        }
        let elapsed_ms = now.duration_since(self.started).as_millis() as u64;
        let elapsed = elapsed_ms as f32 / 1000.0;
        let dur_s = dur_ms as f32 / 1000.0;

        // Playhead position and (for finite modes) the moment playback ends.
        let (t_ms, end_s) = match self.loop_mode {
            LoopMode::OneShot => (elapsed_ms.min(dur_ms), Some(dur_s)),
            LoopMode::HoldLast => (elapsed_ms.min(dur_ms), None),
            LoopMode::LoopedInfinite => (elapsed_ms % dur_ms, None),
            LoopMode::Looped => {
                let total = dur_ms * self.loop_count.max(1) as u64;
                if elapsed_ms >= total {
                    (dur_ms, Some(total as f32 / 1000.0))
                } else {
                    (elapsed_ms % dur_ms, Some(total as f32 / 1000.0))
                }
            }
        };
        self.player.seek(t_ms as u32);

        // HoldLast announces its natural end (AfterLast chains) but keeps
        // holding the last frame until stopped.
        if self.loop_mode == LoopMode::HoldLast && elapsed_ms >= dur_ms && !self.finished_reported {
            self.finished_reported = true;
            finished.push(self.qid);
        }

        let mut weight = 1.0f32;
        if self.fade_in > 0.0 && elapsed < self.fade_in {
            weight *= curve((elapsed / self.fade_in).clamp(0.0, 1.0), self.fade_type);
        }
        if let Some(end) = end_s {
            if elapsed >= end {
                if !self.finished_reported {
                    self.finished_reported = true;
                    finished.push(self.qid);
                }
                return None;
            }
            if self.fade_out > 0.0 {
                let fade_start = (end - self.fade_out).max(0.0);
                if elapsed >= fade_start {
                    let t = (elapsed - fade_start) / (end - fade_start).max(f32::EPSILON);
                    weight *= 1.0 - curve(t.clamp(0.0, 1.0), self.fade_type);
                }
            }
        }
        if let Some((start, dur, ftype)) = self.stopping {
            let t = now.duration_since(start).as_secs_f32();
            if t >= dur {
                if !self.finished_reported {
                    self.finished_reported = true;
                    finished.push(self.qid);
                }
                return None;
            }
            weight *= 1.0 - curve((t / dur.max(f32::EPSILON)).clamp(0.0, 1.0), ftype);
        }
        Some(weight)
    }
}

#[derive(Default)]
pub struct LightingEngine {
    /// One paced sender per distinct destination IP ("" = protocol default).
    senders: BTreeMap<String, DmxSender>,
    /// (protocol, sorted dest set, fps bits) the senders were built from.
    applied: Option<(LightingProtocol, Vec<String>, u32)>,
    live: BTreeMap<FixtureId, FixtureLook>,
    fades: BTreeMap<FixtureId, ActiveFade>,
    /// Playing recorded shows, in start order (composite tie-break).
    shows: Vec<ActiveShow>,
    /// Held overlay layer (recorder monitor / live input bridge): the owner
    /// re-pushes it every tick while active and clears it when done.
    overlay: Option<(u8, MaskedFrame)>,
    /// Shows that ended since the last drain (AfterLast chaining).
    finished_shows: Vec<Decimal>,
    /// Latest sampled canvas pixels per pixel-map segment: (cols, rows, RGBA).
    segment_pixels: BTreeMap<u32, (u32, u32, Vec<u8>)>,
    /// Live state changed since the last submit.
    dirty: bool,
    last_tick: Option<Instant>,
}

/// Same curve family as the audio `FadeProcessor`.
fn curve(t: f32, fade_type: FadeType) -> f32 {
    match fade_type {
        FadeType::Linear => t,
        FadeType::Square => t * t,
        FadeType::InverseSquare => t.sqrt(),
        FadeType::SCurve => t * t * (3.0 - 2.0 * t),
    }
}

impl LightingEngine {
    pub fn is_active(&self) -> bool {
        !self.fades.is_empty() || !self.shows.is_empty() || self.overlay.is_some()
    }

    pub fn active_show_qids(&self) -> impl Iterator<Item = Decimal> + '_ {
        self.shows.iter().map(|show| show.qid)
    }

    /// Fire a lighting cue: crossfade live state to `snapshot` over `fade_time`.
    pub fn go(
        &mut self,
        snapshot: &BTreeMap<FixtureId, FixtureLook>,
        fade_time: f32,
        fade_type: FadeType,
    ) {
        self.go_at(snapshot, fade_time, fade_type, Instant::now());
    }

    fn go_at(
        &mut self,
        snapshot: &BTreeMap<FixtureId, FixtureLook>,
        fade_time: f32,
        fade_type: FadeType,
        now: Instant,
    ) {
        let from = self.evaluate(now);
        for (id, look) in snapshot {
            if fade_time > 0.0 {
                self.fades.insert(
                    *id,
                    ActiveFade {
                        start: now,
                        duration: fade_time,
                        fade_type,
                        from: from.get(id).copied().unwrap_or_default(),
                        to: *look,
                    },
                );
            } else {
                self.live.insert(*id, *look);
                self.fades.remove(id);
            }
        }
        self.dirty = true;
    }

    /// Start (or restart) a recorded DMX show for a cue.
    #[allow(clippy::too_many_arguments)]
    pub fn go_show(
        &mut self,
        qid: Decimal,
        events: Vec<RecEvent>,
        priority: u8,
        fade_in: f32,
        fade_out: f32,
        fade_type: FadeType,
        loop_mode: LoopMode,
        loop_count: i32,
    ) {
        self.shows.retain(|s| s.qid != qid); // refire restarts
        self.shows.push(ActiveShow {
            qid,
            player: ShowPlayer::new(events),
            started: Instant::now(),
            priority,
            loop_mode,
            loop_count,
            fade_in,
            fade_out,
            fade_type,
            stopping: None,
            finished_reported: false,
        });
        self.dirty = true;
    }

    /// Fade out and release a playing show. Returns false if `qid` isn't playing.
    pub fn stop_show(&mut self, qid: Decimal, fade_out_time: f32, fade_type: FadeType) -> bool {
        let Some(show) = self.shows.iter_mut().find(|s| s.qid == qid) else {
            return false;
        };
        if fade_out_time > 0.0 {
            if show.stopping.is_none() {
                show.stopping = Some((Instant::now(), fade_out_time, fade_type));
            }
        } else {
            show.stopping = Some((Instant::now(), 0.0, fade_type));
        }
        self.dirty = true;
        true
    }

    /// Drop all shows immediately (panic stop / project close).
    pub fn stop_all_shows(&mut self) {
        if !self.shows.is_empty() {
            self.shows.clear();
            self.dirty = true;
        }
    }

    /// Shows that ended (naturally or via stop) since the last call — the
    /// caller fires AfterLast chains from these.
    pub fn take_finished_shows(&mut self) -> Vec<Decimal> {
        std::mem::take(&mut self.finished_shows)
    }

    /// Set or clear the overlay layer (recorder monitor / live input bridge).
    /// Composited over the project-level destination like a recorded show.
    pub fn set_overlay(&mut self, overlay: Option<(u8, MaskedFrame)>) {
        if overlay.is_some() || self.overlay.is_some() {
            self.dirty = true;
        }
        self.overlay = overlay;
    }

    /// Release a segment's addresses: with no cached pixels the overlay skips
    /// it entirely, so cue looks on those channels come through again.
    pub fn clear_segment_pixels(&mut self, id: u32) {
        if self.segment_pixels.remove(&id).is_some() {
            self.dirty = true;
        }
    }

    /// Feed freshly sampled canvas pixels for one pixel-map segment.
    pub fn set_segment_pixels(&mut self, id: u32, cols: u32, rows: u32, rgba: Vec<u8>) {
        self.segment_pixels.insert(id, (cols, rows, rgba));
        self.dirty = true;
    }

    /// Cancel all fixture fades, holding their current mid-fade looks.
    pub fn stop_fade(&mut self) {
        self.stop_fade_at(Instant::now());
    }

    fn stop_fade_at(&mut self, now: Instant) {
        if !self.fades.is_empty() {
            self.evaluate(now);
            self.fades.clear();
            self.dirty = true;
        }
    }

    /// Sample current looks into live state, retiring each completed fade.
    fn evaluate(&mut self, now: Instant) -> BTreeMap<FixtureId, FixtureLook> {
        self.fades.retain(|id, f| {
            let elapsed = now.duration_since(f.start).as_secs_f32();
            let complete = elapsed >= f.duration;
            let look = if complete {
                f.to
            } else {
                f.from.lerp(&f.to, curve(elapsed / f.duration, f.fade_type))
            };
            self.live.insert(*id, look);
            !complete
        });
        self.live.clone()
    }

    /// Periodic tick from the main loop: manages the sender lifecycle, advances
    /// fades, and submits a frame when the state changed. Self-throttled.
    pub fn tick(&mut self, cfg: &LightingConfig) {
        // ~60 Hz cap — about_to_wait runs far hotter under ControlFlow::Poll.
        let now = Instant::now();
        if let Some(last) = self.last_tick
            && now.duration_since(last).as_secs_f32() < 1.0 / 60.0
        {
            return;
        }
        self.last_tick = Some(now);

        self.reconcile_sender(cfg);
        if self.senders.is_empty() {
            return;
        }

        let animating = !self.fades.is_empty() || !self.shows.is_empty();
        if !animating && !self.dirty {
            return; // DmxSender keep-alive re-sends the latest frame.
        }

        let looks = self.evaluate(now);

        // Look layer, one masked frame per destination — fixtures render into
        // their node's frame and own exactly the channels they write.
        let mut look_frames: BTreeMap<String, MaskedFrame> = BTreeMap::new();
        for fixture in &cfg.fixtures {
            let Some(profile) = cfg.profile(&fixture.profile_id) else {
                continue;
            };
            let look = looks.get(&fixture.id).copied().unwrap_or_default();
            let bytes = render_look(&profile, &look);
            let dest = fixture.effective_dest(&cfg.dest_ip);
            look_frames.entry(dest.to_string()).or_default().write_span(
                fixture.universe,
                fixture.address,
                &bytes,
            );
        }

        // Pixel-map segments overlay their channels over cue looks. They go to
        // the project-level destination (no per-segment override yet).
        // Drop cached pixels for segments that are gone *or disabled*: a
        // disabled segment must release its addresses back to the cue looks,
        // and re-enabling one must not replay a frame from minutes ago.
        self.segment_pixels
            .retain(|id, _| cfg.active_segments().any(|s| s.id == *id));
        for seg in cfg.active_segments() {
            let Some((cols, rows, rgba)) = self.segment_pixels.get(&seg.id) else {
                continue;
            };
            let Some(profile) = cfg.profile(&seg.profile_id) else {
                continue;
            };
            let pixels = demux_tile(rgba, *cols, [0, 0], [*cols, *rows], seg.order);
            let mut bytes = Vec::with_capacity(pixels.len() * profile.footprint());
            for p in pixels {
                // Sampler delivers RGBA; the colour pipeline expects BGRA.
                let bgra = [p[2], p[1], p[0], p[3]];
                bytes.extend(color_pipeline(bgra, seg.gamma, &seg.color, &profile));
            }
            look_frames
                .entry(cfg.dest_ip.trim().to_string())
                .or_default()
                .pack_fixtures(profile.footprint(), &bytes, seg.universe, seg.address);
        }

        // Advance recorded shows: playhead + fade weight; None = released.
        let mut weights: Vec<Option<f32>> = Vec::with_capacity(self.shows.len());
        for show in &mut self.shows {
            weights.push(show.eval(now, &mut self.finished_shows));
        }

        // Composite per destination. Recorded shows play to the project-level
        // destination; looks go wherever their fixture is patched. Every
        // sender gets a frame (empty if its last fixture moved away) so
        // keep-alive never re-sends levels for a source no longer present.
        let default_dest = cfg.dest_ip.trim();
        for (dest, sender) in &self.senders {
            let mut layers: Vec<(u8, f32, &MaskedFrame)> = Vec::new();
            if let Some(lf) = look_frames.get(dest) {
                layers.push((cfg.look_priority, 1.0, lf));
            }
            if dest == default_dest {
                for (show, w) in self.shows.iter().zip(&weights) {
                    if let Some(w) = w {
                        layers.push((show.priority, *w, show.player.frame()));
                    }
                }
                if let Some((priority, frame)) = &self.overlay {
                    layers.push((*priority, 1.0, frame));
                }
            }
            sender.submit(composite(&layers));
        }

        // Drop released shows (weights and shows are index-aligned).
        let mut i = 0;
        self.shows.retain(|_| {
            let keep = weights[i].is_some();
            i += 1;
            keep
        });
        self.dirty = false;
    }

    /// Build/tear down/rebuild the DMX senders to match the config — one per
    /// distinct destination (project-level + per-fixture overrides).
    fn reconcile_sender(&mut self, cfg: &LightingConfig) {
        let wanted = cfg.enabled.then(|| {
            let mut dests: Vec<String> = cfg
                .fixtures
                .iter()
                .map(|f| f.effective_dest(&cfg.dest_ip).to_string())
                .chain(std::iter::once(cfg.dest_ip.trim().to_string()))
                .collect();
            dests.sort();
            dests.dedup();
            (cfg.protocol, dests, cfg.fps.to_bits())
        });
        if wanted == self.applied {
            return;
        }
        for sender in std::mem::take(&mut self.senders).into_values() {
            sender.shutdown();
        }
        self.applied = wanted.clone();
        let Some((protocol, dests, _)) = wanted else {
            return;
        };

        for dest_ip in dests {
            let dest = match dest_ip.as_str() {
                "" => match protocol {
                    LightingProtocol::Sacn => Dest::Multicast,
                    LightingProtocol::ArtNet => Dest::Broadcast,
                },
                ip => match ip.parse() {
                    Ok(addr) => Dest::Unicast(addr),
                    Err(_) => {
                        log::error!("Lighting: invalid destination IP '{ip}'");
                        continue;
                    }
                },
            };
            log::info!("Lighting output: {protocol:?} > {dest:?} @ {} fps", cfg.fps);
            let transport: std::io::Result<Box<dyn DmxTransport>> = match protocol {
                LightingProtocol::Sacn => {
                    SacnTransport::new(dest, 100, "CuePool").map(|t| Box::new(t) as _)
                }
                LightingProtocol::ArtNet => ArtNetTransport::new(dest).map(|t| Box::new(t) as _),
            };
            match transport {
                Ok(t) => {
                    self.senders.insert(dest_ip, DmxSender::spawn(t, cfg.fps));
                    self.dirty = true; // push current state to the new output
                }
                Err(e) => log::error!("Lighting output failed to start: {e}"),
            }
        }
    }

    /// Drop the senders (project close/reload). Live state resets.
    pub fn shutdown(&mut self) {
        for sender in std::mem::take(&mut self.senders).into_values() {
            sender.shutdown();
        }
        self.applied = None;
        self.live.clear();
        self.fades.clear();
        self.shows.clear();
        self.finished_shows.clear();
        self.overlay = None;
        self.segment_pixels.clear();
        self.dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn look(dimmer: f32) -> FixtureLook {
        FixtureLook {
            dimmer,
            ..Default::default()
        }
    }

    fn assert_dimmers(eng: &mut LightingEngine, now: Instant, expected: &[(FixtureId, f32)]) {
        let state = eng.evaluate(now);
        for (id, expected) in expected {
            let actual = state.get(id).expect("fixture has a look").dimmer;
            assert!(
                (actual - expected).abs() < 1e-5,
                "fixture {id}: expected {expected}, got {actual}"
            );
        }
    }

    #[test]
    fn disjoint_cue_leaves_earlier_fade_running() {
        let mut eng = LightingEngine::default();
        let start = Instant::now();
        eng.go_at(
            &BTreeMap::from([(4, look(1.0))]),
            30.0,
            FadeType::Linear,
            start,
        );
        // #43: the deck is 52/255 when the movers cue starts 6.19s later.
        let movers_start = start + Duration::from_millis(6190);
        let profile = LightingConfig::default().profile("dimmer").unwrap();
        assert_eq!(
            render_look(&profile, &eng.evaluate(movers_start)[&4]),
            vec![52]
        );
        eng.go_at(
            &BTreeMap::from([(1, look(1.0)), (2, look(0.5)), (3, look(0.25))]),
            30.0,
            FadeType::Linear,
            movers_start,
        );
        assert_dimmers(
            &mut eng,
            start + Duration::from_secs(15),
            &[(4, 0.5), (1, 8.81 / 30.0)],
        );
        assert_dimmers(
            &mut eng,
            start + Duration::from_secs(30),
            &[(4, 1.0), (1, 23.81 / 30.0)],
        );
        assert_eq!(
            render_look(&profile, &eng.evaluate(start + Duration::from_secs(30))[&4]),
            vec![255],
        );
        assert_dimmers(
            &mut eng,
            movers_start + Duration::from_secs(30),
            &[(4, 1.0), (1, 1.0), (2, 0.5), (3, 0.25)],
        );
    }

    #[test]
    fn simultaneous_cues_both_fade() {
        let mut eng = LightingEngine::default();
        let start = Instant::now();
        eng.go_at(
            &BTreeMap::from([(1, look(1.0))]),
            10.0,
            FadeType::Linear,
            start,
        );
        eng.go_at(
            &BTreeMap::from([(2, look(0.5))]),
            10.0,
            FadeType::Linear,
            start,
        );
        assert_dimmers(&mut eng, start, &[(1, 0.0), (2, 0.0)]);
        assert_dimmers(
            &mut eng,
            start + Duration::from_secs(5),
            &[(1, 0.5), (2, 0.25)],
        );
        assert_dimmers(
            &mut eng,
            start + Duration::from_secs(10),
            &[(1, 1.0), (2, 0.5)],
        );
    }

    #[test]
    fn live_push_replaces_only_addressed_fixtures() {
        let mut eng = LightingEngine::default();
        let start = Instant::now();
        eng.go_at(
            &BTreeMap::from([(1, look(1.0)), (2, look(1.0))]),
            10.0,
            FadeType::Linear,
            start,
        );
        // The inspector sends a zero-time go for its edited snapshot.
        eng.go_at(
            &BTreeMap::from([(2, look(0.25)), (3, look(0.75))]),
            0.0,
            FadeType::Linear,
            start + Duration::from_secs(2),
        );
        assert_dimmers(
            &mut eng,
            start + Duration::from_secs(2),
            &[(1, 0.2), (2, 0.25), (3, 0.75)],
        );
        assert_dimmers(
            &mut eng,
            start + Duration::from_secs(10),
            &[(1, 1.0), (2, 0.25), (3, 0.75)],
        );
    }

    #[test]
    fn same_fixture_takeover_starts_at_current_look() {
        let mut eng = LightingEngine::default();
        let start = Instant::now();
        eng.go_at(
            &BTreeMap::from([(1, look(1.0))]),
            10.0,
            FadeType::Linear,
            start,
        );
        let takeover = start + Duration::from_secs(4);
        // No tick between the two cues: go must sample the old fade itself.
        eng.go_at(
            &BTreeMap::from([(1, look(0.0))]),
            6.0,
            FadeType::Linear,
            takeover,
        );
        assert_dimmers(&mut eng, takeover, &[(1, 0.4)]);
        assert_dimmers(&mut eng, takeover + Duration::from_secs(3), &[(1, 0.2)]);
        assert_dimmers(&mut eng, takeover + Duration::from_secs(6), &[(1, 0.0)]);
    }

    #[test]
    fn fixture_curves_and_completion_are_independent() {
        let mut eng = LightingEngine::default();
        let start = Instant::now();
        let moving = FixtureLook {
            dimmer: 1.0,
            color: [1.0, 0.5, 0.25],
            pan: 1.0,
            tilt: 0.0,
            strobe: 1.0,
            gobo: 42,
            ..Default::default()
        };
        for (id, duration, curve, target) in [
            (1, 4.0, FadeType::Linear, look(1.0)),
            (2, 8.0, FadeType::Square, moving),
            (3, 16.0, FadeType::InverseSquare, look(1.0)),
            (4, 16.0, FadeType::SCurve, look(1.0)),
        ] {
            eng.go_at(&BTreeMap::from([(id, target)]), duration, curve, start);
        }
        assert!(eng.is_active());
        let halfway = start + Duration::from_secs(4);
        assert_dimmers(
            &mut eng,
            halfway,
            &[(1, 1.0), (2, 0.25), (3, 0.5), (4, 0.15625)],
        );
        assert!(
            !eng.fades.contains_key(&1),
            "completed fixture holds its target"
        );
        assert!(eng.is_active(), "other fixtures are still fading");
        assert_eq!(
            eng.evaluate(halfway)[&2],
            FixtureLook {
                dimmer: 0.25,
                color: [0.25, 0.125, 0.0625],
                pan: 0.625,
                tilt: 0.375,
                ..Default::default()
            },
            "continuous parameters fade; strobe and gobo wait until the end",
        );
        assert_eq!(eng.evaluate(start + Duration::from_secs(8))[&2], moving);
        assert!(eng.is_active());
        assert_dimmers(
            &mut eng,
            start + Duration::from_secs(16),
            &[(1, 1.0), (2, 1.0), (3, 1.0), (4, 1.0)],
        );
        assert!(!eng.is_active(), "the last fade has finished");
        assert_eq!(eng.evaluate(start + Duration::from_secs(30))[&2], moving);
    }

    #[test]
    fn empty_snapshot_leaves_fades_running() {
        let mut eng = LightingEngine::default();
        let start = Instant::now();
        eng.go_at(
            &BTreeMap::from([(1, look(1.0))]),
            10.0,
            FadeType::Linear,
            start,
        );
        for fade_time in [0.0, 5.0] {
            eng.go_at(
                &BTreeMap::new(),
                fade_time,
                FadeType::Square,
                start + Duration::from_secs(2),
            );
        }
        assert!(eng.is_active());
        assert_dimmers(&mut eng, start + Duration::from_secs(10), &[(1, 1.0)]);
        assert!(!eng.is_active());
    }

    #[test]
    fn stop_holds_all_fixtures_without_stopping_shows_or_overlay() {
        let mut eng = LightingEngine::default();
        let start = Instant::now();
        eng.shows.push(show(LoopMode::LoopedInfinite, 0.0, 0));
        eng.set_overlay(Some((150, MaskedFrame::default())));
        eng.go_at(
            &BTreeMap::from([(1, look(1.0))]),
            10.0,
            FadeType::Linear,
            start,
        );
        eng.go_at(
            &BTreeMap::from([(2, look(0.8))]),
            20.0,
            FadeType::Linear,
            start + Duration::from_secs(2),
        );
        eng.dirty = false;
        eng.stop_fade_at(start + Duration::from_secs(5));
        assert!(eng.dirty, "the held looks must be submitted");
        assert!(eng.fades.is_empty());
        assert_dimmers(
            &mut eng,
            start + Duration::from_secs(30),
            &[(1, 0.5), (2, 0.12)],
        );
        assert_eq!(
            eng.active_show_qids().collect::<Vec<_>>(),
            vec![Decimal::ONE]
        );
        assert!(eng.shows[0].stopping.is_none());
        assert!(eng.take_finished_shows().is_empty());
        assert!(eng.is_active(), "recorded show and overlay remain active");
        eng.stop_all_shows();
        assert!(eng.is_active(), "overlay alone remains active");
        eng.set_overlay(None);
        assert!(!eng.is_active());
    }

    fn show(loop_mode: LoopMode, fade_out: f32, age_ms: u64) -> ActiveShow {
        // Two events: ch0 = 100 at t=0, ch0 = 200 at t=1000 → duration 1s.
        let events = vec![
            RecEvent {
                t_ms: 0,
                universe: 1,
                channel: 0,
                value: 100,
            },
            RecEvent {
                t_ms: 1000,
                universe: 1,
                channel: 0,
                value: 200,
            },
        ];
        ActiveShow {
            qid: Decimal::ONE,
            player: ShowPlayer::new(events),
            started: Instant::now() - std::time::Duration::from_millis(age_ms),
            priority: 100,
            loop_mode,
            loop_count: 1,
            fade_in: 0.0,
            fade_out,
            fade_type: FadeType::Linear,
            stopping: None,
            finished_reported: false,
        }
    }

    #[test]
    fn show_oneshot_plays_and_releases_at_end() {
        let mut fin = Vec::new();
        let mut s = show(LoopMode::OneShot, 0.0, 500);
        assert_eq!(s.eval(Instant::now(), &mut fin), Some(1.0));
        assert_eq!(s.player.frame().get(1).unwrap().values()[0], 100);
        assert!(fin.is_empty());

        let mut s = show(LoopMode::OneShot, 0.0, 1500);
        assert_eq!(s.eval(Instant::now(), &mut fin), None, "released past end");
        assert_eq!(fin, vec![Decimal::ONE], "natural end reported once");
    }

    #[test]
    fn show_holdlast_holds_but_reports_finished() {
        let mut fin = Vec::new();
        let mut s = show(LoopMode::HoldLast, 0.0, 5000);
        assert_eq!(s.eval(Instant::now(), &mut fin), Some(1.0), "holds forever");
        assert_eq!(
            s.player.frame().get(1).unwrap().values()[0],
            200,
            "last frame"
        );
        assert_eq!(fin, vec![Decimal::ONE]);
        // Second eval must not re-report.
        s.eval(Instant::now(), &mut fin);
        assert_eq!(fin.len(), 1);
    }

    #[test]
    fn show_infinite_loop_wraps_playhead() {
        let mut fin = Vec::new();
        // 1s show, 1.5s old → wrapped to t=0.5s, before the t=1s event.
        let mut s = show(LoopMode::LoopedInfinite, 0.0, 1500);
        assert_eq!(s.eval(Instant::now(), &mut fin), Some(1.0));
        assert_eq!(s.player.frame().get(1).unwrap().values()[0], 100);
        assert!(fin.is_empty(), "infinite loop never finishes");
    }

    #[test]
    fn show_tail_fade_out_weight() {
        let mut fin = Vec::new();
        // fade_out spans the whole 1s show; at t=0.5 the weight is ~0.5.
        let mut s = show(LoopMode::OneShot, 1.0, 500);
        let w = s.eval(Instant::now(), &mut fin).expect("still live");
        assert!((w - 0.5).abs() < 0.05, "mid tail-fade weight ~0.5, got {w}");
    }

    #[test]
    fn show_stop_fades_then_releases() {
        let mut fin = Vec::new();
        let mut s = show(LoopMode::LoopedInfinite, 0.0, 500);
        // Stop began 1s ago with a 2s fade → half released.
        s.stopping = Some((
            Instant::now() - std::time::Duration::from_secs(1),
            2.0,
            FadeType::Linear,
        ));
        let w = s.eval(Instant::now(), &mut fin).expect("mid stop-fade");
        assert!((w - 0.5).abs() < 0.05, "stop fade half done, got {w}");

        // Stop fade complete → released + finished.
        s.stopping = Some((
            Instant::now() - std::time::Duration::from_secs(3),
            2.0,
            FadeType::Linear,
        ));
        assert_eq!(s.eval(Instant::now(), &mut fin), None);
        assert_eq!(fin, vec![Decimal::ONE]);
    }

    #[test]
    fn go_show_refire_replaces_and_stop_all_clears() {
        let mut eng = LightingEngine::default();
        let ev = vec![RecEvent {
            t_ms: 1000,
            universe: 1,
            channel: 0,
            value: 9,
        }];
        eng.go_show(
            Decimal::ONE,
            ev.clone(),
            100,
            0.0,
            0.0,
            FadeType::Linear,
            LoopMode::OneShot,
            1,
        );
        eng.go_show(
            Decimal::ONE,
            ev,
            120,
            0.0,
            0.0,
            FadeType::Linear,
            LoopMode::OneShot,
            1,
        );
        assert_eq!(eng.shows.len(), 1, "refire replaces, not stacks");
        assert_eq!(eng.shows[0].priority, 120);
        assert!(
            !eng.stop_show(Decimal::TWO, 0.0, FadeType::Linear),
            "unknown qid"
        );
        eng.stop_all_shows();
        assert!(eng.shows.is_empty());
    }

    #[test]
    fn go_snap_and_fade_evaluate() {
        let mut eng = LightingEngine::default();
        // Snap (fade 0): live is the snapshot immediately.
        let mut snap = BTreeMap::new();
        snap.insert(1, look(1.0));
        eng.go(&snap, 0.0, FadeType::Linear);
        assert_eq!(eng.evaluate(Instant::now()).get(&1).unwrap().dimmer, 1.0);

        // Fade to 0 over 10s: shortly after go, still near 1.0.
        let mut snap2 = BTreeMap::new();
        snap2.insert(1, look(0.0));
        eng.go(&snap2, 10.0, FadeType::Linear);
        let early = eng.evaluate(Instant::now()).get(&1).unwrap().dimmer;
        assert!(early > 0.9, "fade barely started, got {early}");
        // Evaluating at fade end lands on the target.
        let end = eng
            .evaluate(Instant::now() + std::time::Duration::from_secs(11))
            .get(&1)
            .unwrap()
            .dimmer;
        assert_eq!(end, 0.0);
    }

    #[test]
    fn absent_fixture_tracks_through_cue() {
        let mut eng = LightingEngine::default();
        let mut a = BTreeMap::new();
        a.insert(1, look(0.8));
        eng.go(&a, 0.0, FadeType::Linear);
        // Second cue only touches fixture 2 — fixture 1 must hold 0.8.
        let mut b = BTreeMap::new();
        b.insert(2, look(0.5));
        eng.go(&b, 0.0, FadeType::Linear);
        let state = eng.evaluate(Instant::now());
        assert_eq!(state.get(&1).unwrap().dimmer, 0.8);
        assert_eq!(state.get(&2).unwrap().dimmer, 0.5);
    }

    #[test]
    fn stop_fade_holds_midpoint() {
        let mut eng = LightingEngine::default();
        let mut a = BTreeMap::new();
        a.insert(1, look(1.0));
        eng.go(&a, 0.0, FadeType::Linear);
        let mut b = BTreeMap::new();
        b.insert(1, look(0.0));
        eng.go(&b, 100.0, FadeType::Linear); // glacial fade
        eng.stop_fade();
        let held = eng.evaluate(Instant::now()).get(&1).unwrap().dimmer;
        assert!(held > 0.95, "held near start value, got {held}");
        assert!(eng.fades.is_empty());
    }
}
