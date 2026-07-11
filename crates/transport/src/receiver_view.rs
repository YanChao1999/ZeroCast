//! Receiver path that decodes H.264 and shows a minifb window.

use crate::decoder;
use crate::display::{self, RgbFrame};
use crate::recv_qos::SharedRecvQoS;
use crate::Receiver;
use anyhow::Result;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

struct DecodeJob {
    annex_b: Vec<u8>,
    width: u32,
    height: u32,
}

fn spawn_decode_thread(
    decode_rx: mpsc::Receiver<DecodeJob>,
    frame_tx: mpsc::SyncSender<RgbFrame>,
    qos: SharedRecvQoS,
    pending: Arc<AtomicU32>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut decode_errors = 0u32;
        let mut frames_ok = 0u64;
        while let Ok(mut job) = decode_rx.recv() {
            pending.fetch_sub(1, Ordering::Relaxed);
            if let Ok(mut w) = qos.lock() {
                w.set_pending_decode(pending.load(Ordering::Relaxed));
            }
            let coalesced = decode_rx.try_iter().count();
            if coalesced > 0 {
                if let Ok(mut w) = qos.lock() {
                    for _ in 0..coalesced {
                        w.record_drop();
                    }
                }
                while let Ok(newer) = decode_rx.try_recv() {
                    job = newer;
                }
            }
            if let Ok(mut w) = qos.lock() {
                w.set_pending_decode(0);
            }
            let decode_start = Instant::now();
            match decoder::decode_access_unit_rgb24(&job.annex_b, job.width, job.height) {
                Ok((rgb24, w, h)) => {
                    let decode_ms = decode_start.elapsed().as_secs_f64() * 1000.0;
                    if let Ok(mut stats) = qos.lock() {
                        stats.record_decode_ms(decode_ms);
                    }
                    let expected = (w as usize) * (h as usize) * 3;
                    if rgb24.len() != expected {
                        continue;
                    }
                    frames_ok += 1;
                    if frames_ok == 1 {
                        eprintln!("receiver: first frame decoded ({w}x{h}, {:.0} ms)", decode_ms);
                    }
                    let frame = RgbFrame {
                        width: w,
                        height: h,
                        rgb24,
                    };
                    if frame_tx.try_send(frame).is_err() {
                        if let Ok(mut stats) = qos.lock() {
                            stats.record_drop();
                        }
                    }
                }
                Err(e) => {
                    decode_errors += 1;
                    if decode_errors <= 8 {
                        eprintln!("decode error: {e:#}");
                    }
                }
            }
        }
    })
}

pub async fn run_with_display(
    receiver: Receiver,
    expect_width: u32,
    expect_height: u32,
    sync: Option<std::sync::Arc<crate::AvSyncState>>,
) -> Result<()> {
    let qos = receiver.qos_stats();
    let pending = Arc::new(AtomicU32::new(0));
    let (frame_tx, frame_rx) = mpsc::sync_channel::<RgbFrame>(2);
    let (decode_tx, decode_rx) = mpsc::channel::<DecodeJob>();
    let title = format!("ZeroCast {expect_width}x{expect_height}");

    let decode_handle = spawn_decode_thread(decode_rx, frame_tx, qos.clone(), pending.clone());

    let mut frames_queued = 0u64;
    let decode_tx_rtp = decode_tx.clone();
    let sync = sync;

    let rtp_handle = tokio::spawn(async move {
        receiver
            .run_frame_delivery(move |_ssrc, _rtp_ts, annex_b| {
                if let Some(s) = &sync {
                    s.on_video_frame();
                }
                let job = DecodeJob {
                    annex_b,
                    width: expect_width,
                    height: expect_height,
                };
                if decode_tx_rtp.send(job).is_ok() {
                    frames_queued += 1;
                    let depth = pending.fetch_add(1, Ordering::Relaxed) + 1;
                    if let Ok(mut w) = qos.lock() {
                        w.set_pending_decode(depth);
                    }
                    if frames_queued == 1
                        || frames_queued <= 20
                        || frames_queued % 60 == 0
                    {
                        eprintln!("receiver: queued frame {frames_queued} for decode");
                    }
                }
            })
            .await
    });

    let display_result = tokio::task::spawn_blocking(move || {
        display::run_blocking(frame_rx, expect_width, expect_height, &title)
    })
    .await?;

    rtp_handle.await??;
    drop(decode_tx);
    let _ = decode_handle.join();
    display_result?;
    Ok(())
}
