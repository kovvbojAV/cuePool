mod support;

use cuepool::EngineTrace;
use cuepool_core::{Cue, LoopMode, StopMode, Timespan, TriggerMode};
use cuepool_harness::{HeadlessShowRunner, RunnerTrace};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::fs;
use support::{Fixture, base, dummy, hap_video, sound, video};

fn started(trace: &[RunnerTrace]) -> Vec<i64> {
    trace
        .iter()
        .filter_map(|entry| match entry {
            RunnerTrace::Engine(EngineTrace::CueStarted { qid, .. }) => qid.to_i64(),
            _ => None,
        })
        .collect()
}

fn video_pts(trace: &[RunnerTrace]) -> Vec<f64> {
    trace
        .iter()
        .filter_map(|entry| match entry {
            RunnerTrace::VideoFrame { pts, .. } => Some(*pts),
            _ => None,
        })
        .collect()
}

fn stop(qid: i64, trigger: TriggerMode, fade_out_time: f32, stop_all: bool) -> Cue {
    Cue::Stop {
        base: base(qid, trigger),
        stop_qid: Decimal::ONE,
        stop_mode: StopMode::Immediate,
        fade_out_time,
        fade_type: Default::default(),
        stop_all,
    }
}

fn terminal_stop_all_keeps_show_clock_stopped(fade_out_time: f32) {
    for (trigger, grouped) in [
        (TriggerMode::Go, false),
        (TriggerMode::WithLast, false),
        (TriggerMode::AfterLast, false),
        (TriggerMode::Go, true),
    ] {
        let mut bed = sound(1, TriggerMode::Go);
        bed.base_mut().loop_mode = LoopMode::LoopedInfinite;
        let mut cues = vec![
            bed,
            Cue::TimeCode {
                base: base(2, TriggerMode::Go),
                start_time: Timespan::from_secs_f64(0.05),
                duration: Timespan::ZERO,
            },
            dummy(3, TriggerMode::AfterLast),
        ];
        let stop_qid = if grouped || trigger != TriggerMode::Go {
            cues.push(if grouped {
                Cue::Group {
                    base: base(4, TriggerMode::Go),
                }
            } else {
                dummy(4, TriggerMode::Go)
            });
            5
        } else {
            4
        };
        let mut ending = stop(stop_qid, trigger, fade_out_time, true);
        if grouped {
            ending.base_mut().parent = Some(Decimal::from(4));
        }
        cues.push(ending);
        let mut disabled = dummy(6, TriggerMode::WithLast);
        disabled.base_mut().enabled = false;
        cues.push(disabled);

        let fixture = Fixture::new(cues).unwrap();
        let mut runner = HeadlessShowRunner::open(&fixture.project).unwrap();
        runner.select(Decimal::ONE).unwrap();
        runner.go().unwrap();
        runner.advance_blocks(10).unwrap();
        assert_eq!(started(&runner.take_trace()), vec![1, 3]);
        assert!(!runner.snapshot().active_cues.is_empty());

        runner.select(Decimal::from(4)).unwrap();
        runner.go().unwrap();
        let clock_at_stop = runner.snapshot().show_elapsed_secs;
        assert!(started(&runner.take_trace()).contains(&stop_qid));
        runner.advance_blocks(30).unwrap();
        let snapshot = runner.snapshot();
        let replayed = started(&runner.take_trace()).contains(&3);
        assert_eq!(
            (clock_at_stop, snapshot.show_elapsed_secs, replayed),
            (None, None, false),
            "terminal stop ({trigger:?}, grouped={grouped}) must keep the clock off and timecodes silent"
        );
        assert!(snapshot.active_cues.is_empty());
    }
}

#[test]
fn immediate_terminal_stop_all_keeps_show_clock_stopped() {
    terminal_stop_all_keeps_show_clock_stopped(0.0);
}

#[test]
fn faded_terminal_stop_all_keeps_show_clock_stopped() {
    terminal_stop_all_keeps_show_clock_stopped(0.2);
}

#[test]
fn opening_reset_chains_start_playback_with_show_clock_at_zero() {
    for fade_out_time in [0.0, 0.2] {
        for trigger in [TriggerMode::WithLast, TriggerMode::AfterLast] {
            for grouped in [false, true] {
                let reset_qid = if grouped { 2 } else { 1 };
                let mut reset = stop(reset_qid, TriggerMode::Go, fade_out_time, true);
                let mut feature = sound(reset_qid + 1, trigger);
                feature.base_mut().loop_mode = LoopMode::LoopedInfinite;
                let mut cues = Vec::new();
                if grouped {
                    cues.push(Cue::Group {
                        base: base(1, TriggerMode::Go),
                    });
                    reset.base_mut().parent = Some(Decimal::ONE);
                    feature.base_mut().parent = Some(Decimal::ONE);
                }
                cues.extend([reset, feature]);
                let fixture = Fixture::new(cues).unwrap();
                let mut runner = HeadlessShowRunner::open(&fixture.project).unwrap();
                // Exercise both a fresh show and a RESET during a running show.
                for _ in 0..2 {
                    runner.select(Decimal::ONE).unwrap();
                    runner.go().unwrap();
                    assert_eq!(runner.snapshot().show_elapsed_secs, Some(0.0));
                    assert_eq!(
                        started(&runner.take_trace()),
                        vec![reset_qid, reset_qid + 1]
                    );
                    // Advance past the fade deadline to ensure old cues finishing
                    // cannot stop the new clock or the new playback instance.
                    runner.advance_blocks(30).unwrap();
                    let snapshot = runner.snapshot();
                    assert_eq!(snapshot.show_elapsed_secs, Some(0.3));
                    assert_eq!(snapshot.active_cues.len(), 1);
                    assert_eq!(snapshot.active_cues[0].qid, Decimal::from(reset_qid + 1));
                    runner.take_trace();
                }
            }
        }
    }
}

#[test]
fn targeted_stops_preserve_the_running_show_clock() {
    for fade_out_time in [0.0, 0.2] {
        let mut bed = sound(1, TriggerMode::Go);
        bed.base_mut().loop_mode = LoopMode::LoopedInfinite;
        let fixture =
            Fixture::new(vec![bed, stop(2, TriggerMode::Go, fade_out_time, false)]).unwrap();
        let mut runner = HeadlessShowRunner::open(&fixture.project).unwrap();
        runner.select(Decimal::ONE).unwrap();
        runner.go().unwrap();
        runner.advance_blocks(10).unwrap();
        assert!(!runner.snapshot().active_cues.is_empty());
        runner.select(Decimal::TWO).unwrap();
        runner.go().unwrap();
        assert_eq!(runner.snapshot().show_elapsed_secs, Some(0.1));
        runner.advance_blocks(30).unwrap();
        assert_eq!(runner.snapshot().show_elapsed_secs, Some(0.4));
        assert!(runner.snapshot().active_cues.is_empty());
    }
}

#[test]
fn targeted_stops_preserve_a_stopped_show_clock() {
    for fade_out_time in [0.0, 0.2] {
        for delay in [0.0, 0.1] {
            let mut ending = stop(2, TriggerMode::Go, fade_out_time, false);
            ending.base_mut().delay = Timespan::from_secs_f64(delay);
            let fixture = Fixture::new(vec![sound(1, TriggerMode::Go), ending]).unwrap();
            let mut runner = HeadlessShowRunner::open(&fixture.project).unwrap();
            runner.select(Decimal::TWO).unwrap();
            runner.go().unwrap();
            assert_eq!(runner.snapshot().show_elapsed_secs, None);
            runner.advance_blocks(30).unwrap();
            assert_eq!(runner.snapshot().show_elapsed_secs, None);
            assert_eq!(started(&runner.take_trace()), vec![2]);
        }
    }
}

#[test]
fn preshow_control_chains_keep_timecode_cues_silent() {
    for trigger in [TriggerMode::WithLast, TriggerMode::AfterLast] {
        for grouped in [false, true] {
            for stop_all in [false, true] {
                let mut feature = sound(1, TriggerMode::Go);
                feature.base_mut().loop_mode = LoopMode::LoopedInfinite;
                let mut cues = vec![
                    feature,
                    Cue::TimeCode {
                        base: base(2, TriggerMode::Go),
                        start_time: Timespan::from_secs_f64(0.05),
                        duration: Timespan::ZERO,
                    },
                    dummy(3, TriggerMode::AfterLast),
                    Cue::Group {
                        base: base(4, TriggerMode::Go),
                    },
                    stop(5, TriggerMode::Go, 0.2, false),
                    Cue::Network {
                        base: base(6, trigger),
                        command: "/preshow/start".into(),
                    },
                    Cue::Lighting {
                        base: base(7, trigger),
                        snapshot: Default::default(),
                        fade_time: 0.2,
                        fade_type: Default::default(),
                    },
                    Cue::Network {
                        base: base(8, trigger),
                        command: "/preshow/ready".into(),
                    },
                ];
                if stop_all {
                    let mut ending = stop(9, trigger, 0.2, true);
                    ending.base_mut().delay = Timespan::from_secs_f64(0.5);
                    cues.push(ending);
                }
                if grouped {
                    for cue in &mut cues[4..] {
                        cue.base_mut().parent = Some(Decimal::from(4));
                    }
                }
                let fixture = Fixture::new(cues).unwrap();
                let mut runner = HeadlessShowRunner::open(&fixture.project).unwrap();
                // Re-entering pre-show must be as harmless as the first GO.
                for _ in 0..2 {
                    runner
                        .select(Decimal::from(if grouped { 4 } else { 5 }))
                        .unwrap();
                    runner.go().unwrap();
                    for _ in 0..70 {
                        assert_eq!(
                            runner.snapshot().show_elapsed_secs,
                            None,
                            "pre-show ({trigger:?}, grouped={grouped}, stop_all={stop_all})"
                        );
                        runner.advance_blocks(1).unwrap();
                    }
                    let fired = started(&runner.take_trace());
                    for qid in 5..=if stop_all { 9 } else { 8 } {
                        assert!(fired.contains(&qid), "pre-show cue {qid} did not execute");
                    }
                    assert!(!fired.contains(&3), "show timecode fired during pre-show");
                }

                // The same timecode must fire when actual playback starts.
                runner.select(Decimal::ONE).unwrap();
                runner.go().unwrap();
                assert_eq!(runner.snapshot().show_elapsed_secs, Some(0.0));
                runner.advance_blocks(10).unwrap();
                assert!(started(&runner.take_trace()).contains(&3));

                // Staff returning a running show to pre-show preserve its clock
                // until the delayed Stop All, when present, actually fires.
                runner
                    .select(Decimal::from(if grouped { 4 } else { 5 }))
                    .unwrap();
                runner.go().unwrap();
                assert_eq!(runner.snapshot().show_elapsed_secs, Some(0.1));
                runner.advance_blocks(49).unwrap();
                assert_eq!(runner.snapshot().show_elapsed_secs, Some(0.59));
                runner.advance_blocks(21).unwrap();
                assert_eq!(
                    runner.snapshot().show_elapsed_secs,
                    (!stop_all).then_some(0.8)
                );
                assert!(runner.snapshot().active_cues.is_empty());
            }
        }
    }
}

#[test]
fn go_starts_the_clock_for_content_and_explicit_timecode_only() {
    for (cue_type, starts_clock) in [
        ("SoundCue", true),
        ("VideoCue", true),
        ("ImageCue", true),
        ("TextCue", true),
        ("PixelMapCue", true),
        ("DmxShowCue", true),
        ("TimeCodeCue", true),
        ("GroupCue", false),
        ("DummyCue", false),
        ("StopCue", false),
        ("VolumeCue", false),
        ("NetworkCue", false),
        ("GotoCue", false),
        ("LightingCue", false),
    ] {
        for delay in [0.0, 0.1] {
            let mut cue: Cue = serde_json::from_value(serde_json::json!({
                "$type": cue_type,
                "qid": "1",
                "path": if cue_type == "VideoCue" { "video.y4m" } else { "tone.wav" },
            }))
            .unwrap();
            cue.base_mut().delay = Timespan::from_secs_f64(delay);
            let fixture = Fixture::new(vec![cue]).unwrap();
            let mut runner = HeadlessShowRunner::open(&fixture.project).unwrap();
            runner.select(Decimal::ONE).unwrap();
            runner.go().unwrap();
            assert_eq!(
                runner.snapshot().show_elapsed_secs,
                starts_clock.then_some(0.0),
                "GO on {cue_type} (delay={delay})"
            );
            runner.advance_blocks(15).unwrap();
            assert_eq!(
                runner.snapshot().show_elapsed_secs,
                starts_clock.then_some(0.15),
                "after {cue_type} (delay={delay})"
            );
        }
    }
}

#[test]
fn delayed_control_chains_start_the_clock_when_they_lead_to_playback() {
    for cue_type in [
        "DummyCue",
        "NetworkCue",
        "LightingCue",
        "VolumeCue",
        "StopCue",
    ] {
        for grouped in [false, true] {
            for enabled in [false, true] {
                let mut control: Cue = serde_json::from_value(serde_json::json!({
                    "$type": cue_type,
                    "qid": "2",
                    "stop_qid": "99",
                    "command": "/prepare/playback",
                }))
                .unwrap();
                control.base_mut().delay = Timespan::from_secs_f64(0.1);
                let mut feature = sound(3, TriggerMode::AfterLast);
                feature.base_mut().enabled = enabled;
                let mut cues = Vec::new();
                if grouped {
                    cues.push(Cue::Group {
                        base: base(1, TriggerMode::Go),
                    });
                    control.base_mut().parent = Some(Decimal::ONE);
                    feature.base_mut().parent = Some(Decimal::ONE);
                }
                cues.extend([control, feature]);
                let fixture = Fixture::new(cues).unwrap();
                let mut runner = HeadlessShowRunner::open(&fixture.project).unwrap();
                runner
                    .select(if grouped { Decimal::ONE } else { Decimal::TWO })
                    .unwrap();
                runner.go().unwrap();
                assert_eq!(
                    runner.snapshot().show_elapsed_secs,
                    enabled.then_some(0.0),
                    "delayed {cue_type} (grouped={grouped}, playback enabled={enabled})"
                );
                runner.advance_blocks(9).unwrap();
                assert!(started(&runner.take_trace()).is_empty());
                runner.advance_blocks(6).unwrap();
                assert_eq!(runner.snapshot().show_elapsed_secs, enabled.then_some(0.15));
                assert_eq!(
                    started(&runner.take_trace()),
                    if enabled { vec![2, 3] } else { vec![2] }
                );
                assert_eq!(runner.snapshot().active_cues.len(), usize::from(enabled));
            }
        }
    }
}

#[test]
fn delayed_groups_only_start_the_clock_for_enabled_content() {
    for content in [false, true] {
        for enabled in [false, true] {
            let mut group = Cue::Group {
                base: base(1, TriggerMode::Go),
            };
            group.base_mut().delay = Timespan::from_secs_f64(0.1);
            let mut child = if content {
                sound(2, TriggerMode::Go)
            } else {
                Cue::Network {
                    base: base(2, TriggerMode::Go),
                    command: "/preshow/start".into(),
                }
            };
            child.base_mut().parent = Some(Decimal::ONE);
            child.base_mut().enabled = enabled;
            let fixture = Fixture::new(vec![group, child]).unwrap();
            let mut runner = HeadlessShowRunner::open(&fixture.project).unwrap();
            runner.select(Decimal::ONE).unwrap();
            runner.go().unwrap();
            assert_eq!(
                runner.snapshot().show_elapsed_secs,
                (content && enabled).then_some(0.0)
            );
            runner.advance_blocks(15).unwrap();
            assert_eq!(
                runner.snapshot().show_elapsed_secs,
                (content && enabled).then_some(0.15)
            );
            assert_eq!(started(&runner.take_trace()).contains(&2), enabled);
        }
    }
}

#[test]
fn loads_relative_media_and_runs_with_last_and_after_last() {
    let fixture = Fixture::new(vec![
        sound(1, TriggerMode::Go),
        dummy(2, TriggerMode::WithLast),
        sound(3, TriggerMode::WithLast),
        dummy(4, TriggerMode::AfterLast),
        dummy(5, TriggerMode::AfterLast),
        dummy(6, TriggerMode::Go),
    ])
    .unwrap();
    let mut runner = HeadlessShowRunner::open(&fixture.project).unwrap();
    runner.select(Decimal::ONE).unwrap();
    runner.go().unwrap();

    let initial = runner.take_trace();
    assert_eq!(started(&initial), vec![1, 2, 3]);
    assert!(initial.iter().any(|entry| matches!(
        entry,
        RunnerTrace::SideEffect {
            qid: Some(qid),
            kind: "dummy",
        } if *qid == Decimal::TWO
    )));
    assert_eq!(runner.snapshot().standby_qid, Some(Decimal::from(6)));
    assert!(!started(&initial).contains(&4));

    runner.advance_blocks(100).unwrap();
    let trace = runner.take_trace();
    assert_eq!(started(&trace), vec![4, 5]);
    assert!(runner.snapshot().active_cues.is_empty());
}

#[test]
fn missing_and_malformed_projects_report_the_path() {
    let fixture = Fixture::new(Vec::new()).unwrap();
    let missing = fixture.dir().join("missing.qproj");
    let error = HeadlessShowRunner::open(&missing)
        .err()
        .expect("missing project should fail")
        .to_string();
    assert!(error.contains(missing.to_string_lossy().as_ref()));

    let malformed = fixture.dir().join("broken.qproj");
    fs::write(&malformed, "{not json").unwrap();
    let error = HeadlessShowRunner::open(&malformed)
        .err()
        .expect("malformed project should fail")
        .to_string();
    assert!(error.contains(malformed.to_string_lossy().as_ref()));
}

#[test]
fn video_pts_are_not_early_and_overdue_frames_use_newest_due() {
    let fixture = Fixture::new(vec![video(1, LoopMode::OneShot)]).unwrap();
    let mut runner = HeadlessShowRunner::open(&fixture.project).unwrap();
    runner.select(Decimal::ONE).unwrap();
    runner.go().unwrap();
    runner.take_trace();

    runner.advance_blocks(3).unwrap();
    assert_eq!(video_pts(&runner.take_trace()), vec![0.0]);
    runner.advance_blocks(1).unwrap();
    assert_eq!(video_pts(&runner.take_trace()), vec![0.04]);

    let mut runner = HeadlessShowRunner::open_with_block_frames(&fixture.project, 4_800).unwrap();
    runner.select(Decimal::ONE).unwrap();
    runner.go().unwrap();
    runner.take_trace();
    runner.advance_blocks(1).unwrap();
    assert_eq!(video_pts(&runner.take_trace()), vec![0.08]);
}

#[test]
fn video_seek_resets_pending_frames_and_preserves_pause() {
    let fixture = Fixture::new(vec![video(1, LoopMode::OneShot)]).unwrap();
    let mut runner = HeadlessShowRunner::open(&fixture.project).unwrap();
    runner.select(Decimal::ONE).unwrap();
    runner.go().unwrap();
    runner.advance_blocks(1).unwrap();
    let instance = runner.snapshot().video.as_ref().unwrap().instance_id;
    runner.take_trace();

    runner.seek(instance, 0.12).unwrap();
    runner.advance_blocks(1).unwrap();
    assert!(
        video_pts(&runner.take_trace())
            .iter()
            .all(|pts| *pts >= 0.12)
    );

    runner.pause().unwrap();
    runner.seek(instance, 0.08).unwrap();
    let seek_trace = runner.take_trace();
    assert!(seek_trace.iter().any(|entry| matches!(
        entry,
        RunnerTrace::VideoSeek {
            target_secs,
            paused: true,
            ..
        } if (*target_secs - 0.08).abs() < 0.0001
    )));
    runner.advance_blocks(5).unwrap();
    assert!(video_pts(&runner.take_trace()).is_empty());
    assert!(runner.snapshot().paused);
    assert!(runner.snapshot().video.as_ref().unwrap().paused);

    runner.resume().unwrap();
    runner.advance_blocks(1).unwrap();
    assert_eq!(video_pts(&runner.take_trace()), vec![0.08]);

    runner.pause().unwrap();
    runner.seek(instance, f32::INFINITY).unwrap();
    let snapshot = runner.snapshot();
    assert!(snapshot.video.as_ref().unwrap().position_secs < 0.2);
    assert!(runner.take_trace().iter().any(|entry| matches!(
        entry,
        RunnerTrace::VideoSeek { target_secs, .. }
            if *target_secs < 0.2 && *target_secs > 0.199
    )));
}

#[test]
fn hap_q_uses_the_production_decoder_for_pts_seek_pause_and_eof() {
    let fixture = Fixture::new(vec![hap_video(1, LoopMode::OneShot)]).unwrap();
    let mut runner = HeadlessShowRunner::open(&fixture.project).unwrap();
    runner.select(Decimal::ONE).unwrap();
    runner.go().unwrap();
    runner.advance_blocks(3).unwrap();
    assert_eq!(video_pts(&runner.take_trace()), vec![0.0, 0.02]);

    let instance = runner.snapshot().video.as_ref().unwrap().instance_id;
    runner.pause().unwrap();
    runner.seek(instance, 0.04).unwrap();
    runner.advance_blocks(5).unwrap();
    assert!(video_pts(&runner.take_trace()).is_empty());
    runner.resume().unwrap();
    runner.advance_blocks(1).unwrap();
    assert_eq!(video_pts(&runner.take_trace()), vec![0.04]);

    runner.advance_blocks(20).unwrap();
    assert!(runner.snapshot().video.is_none());
}

#[test]
fn sound_seek_clamps_and_stays_paused() {
    let fixture = Fixture::new(vec![sound(1, TriggerMode::Go)]).unwrap();
    let mut runner = HeadlessShowRunner::open(&fixture.project).unwrap();
    runner.select(Decimal::ONE).unwrap();
    runner.go().unwrap();
    runner.advance_blocks(2).unwrap();
    let instance = runner.snapshot().active_cues[0].instance_id;

    runner.seek(instance, 0.05).unwrap();
    let position = runner.snapshot().active_cues[0].position_secs;
    assert!((position - 0.05).abs() < 0.0001);

    runner.pause().unwrap();
    runner.seek(instance, f32::INFINITY).unwrap();
    let paused = runner.snapshot();
    let position = paused.active_cues[0].position_secs;
    assert!(paused.paused);
    assert!(position < 0.1 && position > 0.099);
    runner.advance_blocks(10).unwrap();
    assert_eq!(runner.snapshot().active_cues[0].position_secs, position);
}

#[test]
fn video_eof_modes_stop_hold_and_reopen_without_stale_epochs() {
    let one_shot = Fixture::new(vec![video(1, LoopMode::OneShot)]).unwrap();
    let mut runner = HeadlessShowRunner::open(&one_shot.project).unwrap();
    runner.select(Decimal::ONE).unwrap();
    runner.go().unwrap();
    runner.advance_blocks(20).unwrap();
    assert!(runner.snapshot().video.is_none());

    let hold = Fixture::new(vec![video(1, LoopMode::HoldLast)]).unwrap();
    let mut runner = HeadlessShowRunner::open(&hold.project).unwrap();
    runner.select(Decimal::ONE).unwrap();
    runner.go().unwrap();
    runner.advance_blocks(20).unwrap();
    assert!(runner.snapshot().video.is_some());
    assert_eq!(video_pts(&runner.take_trace()).last().copied(), Some(0.16));

    let looping = Fixture::new(vec![video(1, LoopMode::LoopedInfinite)]).unwrap();
    let mut runner = HeadlessShowRunner::open(&looping.project).unwrap();
    runner.select(Decimal::ONE).unwrap();
    runner.go().unwrap();
    runner.take_trace();
    runner.advance_blocks(22).unwrap();
    let trace = runner.take_trace();
    let eof_epochs: Vec<u64> = trace
        .iter()
        .filter_map(|entry| match entry {
            RunnerTrace::VideoEof { epoch, .. } => Some(*epoch),
            _ => None,
        })
        .collect();
    assert_eq!(eof_epochs, vec![1]);
    assert_eq!(runner.snapshot().video.as_ref().unwrap().epoch, 2);
    assert!(
        trace
            .iter()
            .any(|entry| matches!(entry, RunnerTrace::VideoFrame { epoch: 2, .. }))
    );
}

#[test]
fn stop_and_project_replacement_clear_runtime_state() {
    let mut delayed = sound(2, TriggerMode::WithLast);
    delayed.base_mut().delay = cuepool_core::Timespan::from_secs_f64(1.0);
    let fixture = Fixture::new(vec![sound(1, TriggerMode::Go), delayed]).unwrap();
    let replacement = Fixture::new(vec![dummy(9, TriggerMode::Go)]).unwrap();
    let mut runner = HeadlessShowRunner::open(&fixture.project).unwrap();
    runner.select(Decimal::ONE).unwrap();
    runner.go().unwrap();
    assert!(!runner.snapshot().active_cues.is_empty());

    runner.stop().unwrap();
    let snapshot = runner.snapshot();
    assert!(snapshot.active_cues.is_empty());
    assert!(snapshot.video.is_none());
    assert!(!snapshot.paused);

    runner.select(Decimal::ONE).unwrap();
    runner.go().unwrap();
    runner.take_trace();
    runner.replace_project(&replacement.project).unwrap();
    runner.advance_blocks(120).unwrap();
    assert!(!started(&runner.take_trace()).contains(&2));
    assert!(runner.snapshot().active_cues.is_empty());
    assert_eq!(runner.snapshot().standby_qid, None);
}
