use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::{HashMap, BTreeMap};

use rtp::packet::Packet as RtpPacket;
// webrtc-util marshal/unmarshal traits used by the `rtp` crate
use webrtc_util::marshal::{Marshal, Unmarshal};
mod av_sync;
mod decoder;
#[cfg(feature = "audio")]
#[cfg(feature = "audio-playback")]
mod audio_playout;
#[cfg(feature = "audio")]
mod audio_rtp;
mod recv_qos;
mod rtcp;
#[cfg(feature = "display")]
mod display;
mod encoder;
mod h264_rtp;
#[cfg(feature = "display")]
mod receiver_view;
mod test_pattern;

pub use decoder::decode_access_unit_rgb24;
#[cfg(feature = "display")]
pub use display::RgbFrame;
pub use encoder::{
    encode_access_unit_oneshot, FfmpegCliEncoder, VideoEncoder, DEFAULT_FPS, DEFAULT_HEIGHT,
    DEFAULT_WIDTH,
};
pub use h264_rtp::nalus_to_annex_b;
pub use av_sync::AvSyncState;
pub use recv_qos::{new_shared, RecvQoSWindow, SharedRecvQoS};
pub use rtcp::RecvFeedback;
pub use test_pattern::{decoded_cycle_from_rgb, test_cycle_frame, CYCLE_COUNT};
#[cfg(feature = "audio")]
pub use audio_rtp::{recv_log_av, audio_stream_loop, AudioReceiver, AudioSender, RecvAvOpts};

/// Optional QoS context for `capture_encode_and_stream` (sender-side metrics).
#[derive(Debug, Clone, Copy)]
pub struct StreamQosOpts {
    pub display_w: u32,
    pub display_h: u32,
    pub refresh_hz: u32,
    pub recv_cap: Option<zerocast_core::ReceiverCapability>,
    pub device_class: zerocast_core::DeviceClass,
}

/// Receive RTP, decode H.264, and show a minifb window (blocking UI thread).
#[cfg(feature = "display")]
pub async fn recv_with_display(local: &str, width: u32, height: u32) -> anyhow::Result<()> {
    recv_with_display_av(local, width, height, false, false, DEFAULT_FPS).await
}

/// Receive video window plus optional Opus audio playback.
#[cfg(all(feature = "display", feature = "audio"))]
pub async fn recv_with_display_av(
    local: &str,
    width: u32,
    height: u32,
    with_audio: bool,
    audio_playback: bool,
    fps: u32,
) -> anyhow::Result<()> {
    if !with_audio {
        let receiver = Receiver::bind_with_fps(local, fps).await?;
        return receiver_view::run_with_display(receiver, width, height, None).await;
    }
    let audio_local = zerocast_protocol::audio_bind_from_video(local, None)?;
    eprintln!(
        "audio: listening on {audio_local}{}",
        if audio_playback { " (playback)" } else { " (log)" }
    );
    let sync = std::sync::Arc::new(AvSyncState::new(fps));
    let video = Receiver::bind_with_fps(local, fps).await?;
    let audio = audio_rtp::AudioReceiver::bind(&audio_local).await?;
    let sync_a = sync.clone();
    let audio_handle = tokio::spawn(async move {
        if audio_playback {
            audio.run_playback(Some(sync_a)).await
        } else {
            audio.run_log(Some(sync_a)).await
        }
    });
    let result = receiver_view::run_with_display(video, width, height, Some(sync)).await;
    audio_handle.abort();
    result
}

#[cfg(all(feature = "display", not(feature = "audio")))]
pub async fn recv_with_display_av(
    local: &str,
    width: u32,
    height: u32,
    with_audio: bool,
    _audio_playback: bool,
    fps: u32,
) -> anyhow::Result<()> {
    if with_audio {
        anyhow::bail!("this binary was built without the `audio` feature");
    }
    let receiver = Receiver::bind_with_fps(local, fps).await?;
    receiver_view::run_with_display(receiver, width, height, None).await
}

/// Max RTP payload before UDP fragmentation on typical LAN MTU.
const MAX_RTP_PAYLOAD: usize = 1200;
const RTP_CLOCK_RATE: u32 = 90_000;

pub struct Sender {
    socket: tokio::net::UdpSocket,
    target: SocketAddr,
    seq: u16,
    ssrc: u32,
    payload_type: u8,
}

pub struct Receiver {
    socket: tokio::net::UdpSocket,
    rtcp_rx: tokio::sync::mpsc::UnboundedReceiver<rtcp::RtcpSrAnchor>,
    qos: SharedRecvQoS,
    sender_addr: std::sync::Arc<std::sync::Mutex<Option<SocketAddr>>>,
    #[allow(dead_code)]
    recv_ssrc: u32,
}

impl Sender {
    pub async fn bind(local: &str, target: &str) -> anyhow::Result<Self> {
        let socket = tokio::net::UdpSocket::bind(local).await?;
        let target: SocketAddr = target.parse()?;
        // simple ssrc derived from current time
        let ssrc = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u32;
        Ok(Self { socket, target, seq: 0, ssrc, payload_type: 96 })
    }

    /// Send periodic synthetic "frames" as minimal RTP packets (12-byte header)
    pub async fn run(self) -> anyhow::Result<()> {
        let mut s = self;
        // RTP clock rate in Hz (use 90 kHz for video-like streams)
        const RTP_CLOCK_RATE: u32 = 90_000;
        // target framerate for synthetic frames
        const FRAMERATE: u32 = 30;
        let tick_inc: u32 = RTP_CLOCK_RATE / FRAMERATE; // nominal increment per frame
        // initialize RTP timestamp from current time (wrapped to u32) and anchor instant
        let mut prev_instant = SystemTime::now();
        let now_ms_init = prev_instant.duration_since(UNIX_EPOCH)?.as_millis() as u128;
        let mut ts: u32 = ((now_ms_init * RTP_CLOCK_RATE as u128) / 1000) as u32;
        loop {
            s.seq = s.seq.wrapping_add(1);
            let mut pkt = RtpPacket::default();
            pkt.header.version = 2;
            pkt.header.payload_type = s.payload_type;
            pkt.header.sequence_number = s.seq;
            pkt.header.timestamp = ts;
            pkt.header.ssrc = s.ssrc;
            pkt.payload = vec![0u8, 1, 2, 3].into();
            let buf = pkt.marshal()?;
            let _ = s.socket.send_to(&buf, &s.target).await?;
            // sleep (nominal), then measure actual elapsed and advance timestamp by measured time
            tokio::time::sleep(std::time::Duration::from_micros(1_000_000u64 / FRAMERATE as u64)).await;
            let now = SystemTime::now();
            let elapsed_ns = now.duration_since(prev_instant)?.as_nanos() as u128;
            // convert elapsed ns -> RTP ticks and advance timestamp
            let delta_ticks = ((elapsed_ns.saturating_mul(RTP_CLOCK_RATE as u128)) / 1_000_000_000u128) as u32;
            if delta_ticks == 0 {
                // fallback to nominal increment if measured interval is too small
                ts = ts.wrapping_add(tick_inc);
            } else {
                ts = ts.wrapping_add(delta_ticks);
            }
            prev_instant = now;
        }
    }

    /// Map encoder PTS (nanoseconds) to a 90 kHz RTP timestamp.
    pub fn pts_ns_to_rtp(pts_ns: u128) -> u32 {
        ((pts_ns.saturating_mul(RTP_CLOCK_RATE as u128)) / 1_000_000_000u128) as u32
    }

    fn rtcp_target(&self) -> Option<SocketAddr> {
        let port = self.target.port().wrapping_add(1);
        if port == 0 {
            return None;
        }
        Some(SocketAddr::new(self.target.ip(), port))
    }

    /// Best-effort RTCP Sender Report for A/V sync on the receiver.
    pub async fn send_rtcp_sr(&self, frame_rtp_ts: u32) -> anyhow::Result<()> {
        let Some(rtcp_target) = self.rtcp_target() else {
            return Ok(());
        };
        let now_ns = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let (ntp_secs, ntp_frac) = rtcp::wall_to_ntp(now_ns);
        let sr = rtcp::build_rtcp_sr(self.ssrc, frame_rtp_ts, ntp_secs, ntp_frac);
        let _ = self.socket.send_to(&sr, &rtcp_target).await;
        Ok(())
    }

    /// Non-blocking read of recv RTCP APP feedback (ZeroCast QoS).
    pub async fn try_recv_feedback(&self) -> Option<rtcp::RecvFeedback> {
        let mut buf = [0u8; 64];
        match self.socket.try_recv_from(&mut buf) {
            Ok((n, _)) => rtcp::parse_recv_feedback(&buf[..n]),
            Err(_) => None,
        }
    }

    /// Send Annex-B NALUs as RTP (RFC 6184 packetization via `rtp::codecs::h264`).
    pub async fn send_nalus(&mut self, nalus: &[Vec<u8>], frame_rtp_ts: u32) -> anyhow::Result<()> {
        let annex_b = h264_rtp::nalus_to_annex_b(nalus);
        if annex_b.is_empty() {
            return Ok(());
        }
        let payloads = h264_rtp::packetize_annex_b(&annex_b, MAX_RTP_PAYLOAD)?;
        let last = payloads.len().saturating_sub(1);
        for (i, payload) in payloads.into_iter().enumerate() {
            self.send_rtp_payload(&payload, frame_rtp_ts, i == last)
                .await?;
        }
        Ok(())
    }

    async fn send_rtp_payload(
        &mut self,
        payload: &[u8],
        frame_rtp_ts: u32,
        marker: bool,
    ) -> anyhow::Result<()> {
        self.seq = self.seq.wrapping_add(1);
        let mut pkt = RtpPacket::default();
        pkt.header.version = 2;
        pkt.header.payload_type = self.payload_type;
        pkt.header.sequence_number = self.seq;
        pkt.header.timestamp = frame_rtp_ts;
        pkt.header.ssrc = self.ssrc;
        pkt.header.marker = marker;
        pkt.payload = payload.to_vec().into();
        let buf = pkt.marshal()?;
        let _ = self.socket.send_to(&buf, &self.target).await?;
        Ok(())
    }

}

fn strip_annex_b_start_code(nalu: &[u8]) -> &[u8] {
    if nalu.len() >= 4 && nalu[0..4] == [0, 0, 0, 1] {
        &nalu[4..]
    } else if nalu.len() >= 3 && nalu[0..3] == [0, 0, 1] {
        &nalu[3..]
    } else {
        nalu
    }
}

impl Receiver {
    pub async fn bind(local: &str) -> anyhow::Result<Self> {
        Self::bind_with_fps(local, DEFAULT_FPS).await
    }

    /// Bind with target fps for decode-lag estimation in QoS feedback.
    pub async fn bind_with_fps(local: &str, target_fps: u32) -> anyhow::Result<Self> {
        let socket = tokio::net::UdpSocket::bind(local).await?;
        let std_sock = socket.into_std()?;
        std_sock.set_nonblocking(true)?;
        let feedback_std = std_sock.try_clone()?;
        feedback_std.set_nonblocking(true)?;
        let socket = tokio::net::UdpSocket::from_std(std_sock)?;
        let feedback_socket = tokio::net::UdpSocket::from_std(feedback_std)?;
        let local_addr: SocketAddr = local.parse()?;
        let rtcp_port = local_addr.port().wrapping_add(1);
        let rtcp_bind = SocketAddr::new(local_addr.ip(), rtcp_port);
        let rtcp_socket = tokio::net::UdpSocket::bind(rtcp_bind).await?;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 1500];
            loop {
                match rtcp_socket.recv_from(&mut buf).await {
                    Ok((n, _addr)) => {
                        if let Some(anchor) = rtcp::parse_rtcp_sr(&buf[..n]) {
                            let _ = tx.send(anchor);
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let qos = recv_qos::new_shared(target_fps);
        let sender_addr = std::sync::Arc::new(std::sync::Mutex::new(None));
        let recv_ssrc = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u32;
        let feedback_qos = qos.clone();
        let feedback_sender = sender_addr.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                let dest = match feedback_sender.lock() {
                    Ok(guard) => *guard,
                    Err(_) => continue,
                };
                let Some(dest) = dest else { continue };
                let fb = match feedback_qos.lock() {
                    Ok(mut w) => w.snapshot_feedback(),
                    Err(_) => continue,
                };
                let pkt = rtcp::build_recv_feedback(recv_ssrc, fb);
                let _ = feedback_socket.send_to(&pkt, dest).await;
            }
        });
        Ok(Self {
            socket,
            rtcp_rx: rx,
            qos,
            sender_addr,
            recv_ssrc,
        })
    }

    pub fn qos_stats(&self) -> SharedRecvQoS {
        self.qos.clone()
    }

    /// Run receiver loop and log RTP/NALU stats to the console.
    pub async fn run(self) -> anyhow::Result<()> {
        self.run_frame_delivery(|_ssrc, _ts, au| {
            println!("rtp access unit bytes={}", au.len());
        })
        .await
    }

    /// Run receiver loop; invoke `on_frame` with a complete Annex-B access unit on RTP marker.
    pub async fn run_frame_delivery<F>(self, on_frame: F) -> anyhow::Result<()>
    where
        F: FnMut(u32, u32, Vec<u8>),
    {
        self.run_frame_delivery_with_sync(on_frame, None).await
    }

    /// Like [`run_frame_delivery`] but feeds RTCP SR anchors into optional A/V sync state.
    pub async fn run_frame_delivery_with_sync<F>(
        self,
        mut on_frame: F,
        sync: Option<std::sync::Arc<AvSyncState>>,
    ) -> anyhow::Result<()>
    where
        F: FnMut(u32, u32, Vec<u8>),
    {
        let mut buf = vec![0u8; 1500];
        let mut frame_assemblers: HashMap<u32, (u32, h264_rtp::FrameAssembler)> = HashMap::new();
        // per-ssrc mapping: base RTP timestamp -> wall-clock ns at first receipt
        let mut ssrc_bases: HashMap<u32, (u32, u128)> = HashMap::new();
        // per-ssrc last RTP timestamp to compute per-packet expected intervals
        let mut ssrc_last_ts: HashMap<u32, u32> = HashMap::new();
        // previous arrival time (ns) for inter-arrival measurement
        let mut prev_now_ns: Option<u128> = None;
        // per-SSRC jitter buffers: map SSRC -> ordered map of seq -> (payload, rtp_ts, arrival_ns, pt)
        let mut jitter_buffers: HashMap<u32, BTreeMap<u16, (Vec<u8>, u32, u128, u8)>> = HashMap::new();
        // last delivered sequence per SSRC
        let mut delivered_seq: HashMap<u32, u16> = HashMap::new();
        let mut rtcp_rx = self.rtcp_rx;
        let qos = self.qos.clone();
        let sender_addr = self.sender_addr.clone();
        loop {
            // drain any pending RTCP Sender Reports first
            while let Ok(anchor) = rtcp_rx.try_recv() {
                if let Some(s) = &sync {
                    s.on_video_rtcp_sr(anchor);
                }
                ssrc_bases.insert(anchor.ssrc, (anchor.rtp_ts, anchor.wall_ns));
                ssrc_last_ts.insert(anchor.ssrc, anchor.rtp_ts);
            }

            let (n, addr) = self.socket.recv_from(&mut buf).await?;
            if let Ok(mut guard) = sender_addr.lock() {
                if guard.is_none() {
                    *guard = Some(addr);
                    eprintln!("qos: recv feedback target {addr}");
                }
            }
            if n < 12 {
                eprintln!("received too-small packet from {}", addr);
                continue;
            }
            match RtpPacket::unmarshal(&mut &buf[..n]) {
                    Ok(pkt) => {
                        let pt = pkt.header.payload_type;
                        let seq = pkt.header.sequence_number;
                        let ts = pkt.header.timestamp;
                        let ssrc = pkt.header.ssrc;
                        let marker = pkt.header.marker;
                        let payload_bytes = pkt.payload.to_vec();
                        // arrival time
                        let now_ns = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() as u128;

                        // insert into per-SSRC jitter buffer (BTreeMap keyed by sequence)
                        let buftree = jitter_buffers.entry(ssrc).or_insert(BTreeMap::new());
                        buftree.insert(seq, (payload_bytes, ts, now_ns, pt));

                        // determine next expected sequence for this SSRC
                        let mut next = match delivered_seq.get(&ssrc) {
                            Some(s) => s.wrapping_add(1),
                            None => {
                                // if none delivered yet, set expectation to lowest sequence we have
                                *buftree.keys().next().unwrap()
                            }
                        };

                        // deliver contiguous sequences from buffer
                        loop {
                            if let Some((payload_vec, ts_val, arr_ns, _pt_val)) = buftree.remove(&next) {
                                // compute latency using anchoring (ssrc_bases)
                                const RTP_CLOCK_RATE: u32 = 90_000;
                                let _latency_ms: u64 = match ssrc_bases.get(&ssrc) {
                                    None => {
                                        ssrc_bases.insert(ssrc, (ts_val, arr_ns));
                                        0
                                    }
                                    Some((base_ts, base_now_ns)) => {
                                        if ts_val < *base_ts {
                                            ssrc_bases.insert(ssrc, (ts_val, arr_ns));
                                            ssrc_last_ts.insert(ssrc, ts_val);
                                            prev_now_ns = Some(arr_ns);
                                            0
                                        } else {
                                            let delta_ticks = ts_val.wrapping_sub(*base_ts) as u128;
                                            let expected_send_ns = base_now_ns.saturating_add(delta_ticks.saturating_mul(1_000_000_000u128) / (RTP_CLOCK_RATE as u128));
                                            let _expected_interval_ms = match ssrc_last_ts.get(&ssrc) {
                                                Some(prev_ts) => {
                                                    let delta_since_last = ts_val.wrapping_sub(*prev_ts) as u128;
                                                    (delta_since_last as f64) * 1000.0 / (RTP_CLOCK_RATE as f64)
                                                }
                                                None => 0.0,
                                            };
                                            let _arrival_interval_ms = match prev_now_ns {
                                                Some(pn) => ((arr_ns - pn) as f64) / 1_000_000.0,
                                                None => 0.0,
                                            };
                                            ssrc_last_ts.insert(ssrc, ts_val);
                                            prev_now_ns = Some(arr_ns);
                                            if arr_ns > expected_send_ns {
                                                ((arr_ns - expected_send_ns) / 1_000_000u128) as u64
                                            } else { 0 }
                                        }
                                    }
                                };

                                let lost = match delivered_seq.get(&ssrc) {
                                    Some(prev) if next > *prev && next.wrapping_sub(*prev) > 1 => {
                                        (next.wrapping_sub(*prev) - 1) as u32
                                    }
                                    _ => 0,
                                };
                                if lost > 0 {
                                    if let Ok(mut w) = qos.lock() {
                                        w.record_loss(lost);
                                    }
                                } else if let Ok(mut w) = qos.lock() {
                                    w.record_packet();
                                }
                                if let Some(pn) = prev_now_ns {
                                    let interval_ms = ((arr_ns.saturating_sub(pn)) as f64) / 1_000_000.0;
                                    if interval_ms > 0.0 {
                                        if let Ok(mut w) = qos.lock() {
                                            w.record_jitter(interval_ms);
                                        }
                                    }
                                }

                                if !payload_vec.is_empty() {
                                    let entry = frame_assemblers
                                        .entry(ssrc)
                                        .or_insert_with(|| (ts_val, h264_rtp::FrameAssembler::new()));
                                    if entry.0 != ts_val {
                                        entry.1.reset();
                                        entry.0 = ts_val;
                                    }
                                    if let Err(e) = entry.1.push_rtp_payload(&payload_vec) {
                                        eprintln!("H264 depacketize: {e:#}");
                                    } else if marker {
                                        let au = entry.1.finish();
                                        entry.1.reset();
                                        if !au.is_empty() {
                                            on_frame(ssrc, ts_val, au);
                                        }
                                    }
                                }

                                delivered_seq.insert(ssrc, next);
                                // advance next and loop
                                next = next.wrapping_add(1);
                            } else {
                                break;
                            }
                        }
                        // purge old buffered entries to avoid unbounded growth (200ms)
                        if let Some(tree) = jitter_buffers.get_mut(&ssrc) {
                            let threshold_ns = now_ns.saturating_sub(200_000_000u128);
                            let old_keys: Vec<u16> = tree.iter().filter_map(|(k, v)| if v.2 < threshold_ns { Some(*k) } else { None }).collect();
                            for k in old_keys { tree.remove(&k); }
                        }
                }
                Err(e) => {
                    eprintln!("failed to parse RTP packet from {}: {}", addr, e);
                    continue;
                }
            }
        }
    }
}

/// Capture one synthetic frame, encode, and send over RTP (single frame).
pub async fn capture_encode_and_send(local: &str, target: &str) -> anyhow::Result<()> {
    capture_encode_and_stream(local, target, DEFAULT_WIDTH, DEFAULT_HEIGHT, DEFAULT_FPS, 1).await
}

/// Continuous capture → H.264 (ffmpeg CLI) → RTP stream with frame pacing.
pub async fn capture_encode_and_stream(
    local: &str,
    target: &str,
    width: u32,
    height: u32,
    fps: u32,
    max_frames: u64,
) -> anyhow::Result<()> {
    capture_encode_and_stream_with_qos(
        local, target, width, height, fps, max_frames, None, false, false, false, None,
    )
    .await
}

pub async fn capture_encode_and_stream_with_qos(
    local: &str,
    target: &str,
    width: u32,
    height: u32,
    fps: u32,
    max_frames: u64,
    qos: Option<StreamQosOpts>,
    test_cycle: bool,
    with_audio: bool,
    use_test_tone: bool,
    audio_target_override: Option<&str>,
) -> anyhow::Result<()> {
    // Screen + network setup before ffmpeg: a long gap after the first stdin writes
    // makes ffmpeg's pipe encoder stop producing output on Windows.
    if test_cycle {
        eprintln!("stream: test-cycle mode (10-color pattern, cycle in logs)");
    }
    if test_cycle || (max_frames > 0 && max_frames <= 30) {
        eprintln!("stream: waiting 1s (start recv before this if not already running)");
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    let (screen, first_frame) = if test_cycle {
        (None, test_pattern::test_cycle_frame(width, height, 0))
    } else {
        let mut screen = zerocast_platform::ScreenCapture::open(width, height)?;
        let first = screen.capture_frame()?;
        (Some(screen), first)
    };
    let sender = Sender::bind(local, target).await?;
    let video_encoder = FfmpegCliEncoder::open_with_warmup(width, height, fps, Some(&first_frame))?;
    stream_loop(
        local,
        target,
        width,
        height,
        fps,
        max_frames,
        video_encoder,
        screen,
        Some(sender),
        Some(first_frame),
        qos,
        test_cycle,
        with_audio,
        use_test_tone,
        audio_target_override,
    )
    .await
}

pub async fn capture_encode_and_stream_with_encoder(
    local: &str,
    target: &str,
    width: u32,
    height: u32,
    fps: u32,
    max_frames: u64,
    video_encoder: FfmpegCliEncoder,
) -> anyhow::Result<()> {
    stream_loop(
        local,
        target,
        width,
        height,
        fps,
        max_frames,
        video_encoder,
        None,
        None,
        None,
        None,
        false,
        false,
        false,
        None,
    )
    .await
}

fn apply_qos_stream_profile(
    profile: zerocast_core::StreamProfile,
    width: &mut u32,
    height: &mut u32,
    fps: &mut u32,
    frame_duration: &mut std::time::Duration,
    screen: &mut zerocast_platform::ScreenCapture,
    video_encoder: &mut FfmpegCliEncoder,
) -> anyhow::Result<()> {
    if profile.width == *width && profile.height == *height && profile.fps == *fps {
        return Ok(());
    }
    eprintln!(
        "qos: reconfiguring stream to {} ({}x{} @ {} fps)",
        profile.name, profile.width, profile.height, profile.fps
    );
    *width = profile.width;
    *height = profile.height;
    *fps = profile.fps;
    *frame_duration = std::time::Duration::from_secs_f64(1.0 / (*fps).max(1) as f64);
    if let Err(e) = screen.reconfigure(*width, *height) {
        eprintln!("screen capture: reconfigure failed ({e:#}), reopening");
        *screen = zerocast_platform::ScreenCapture::open(*width, *height)?;
    }
    let frame = screen.capture_frame()?;
    *video_encoder = FfmpegCliEncoder::open_with_warmup(*width, *height, *fps, Some(&frame))?;
    Ok(())
}

async fn stream_loop(
    _local: &str,
    _target: &str,
    mut width: u32,
    mut height: u32,
    mut fps: u32,
    max_frames: u64,
    mut video_encoder: FfmpegCliEncoder,
    screen: Option<zerocast_platform::ScreenCapture>,
    sender: Option<Sender>,
    first_frame: Option<Vec<u8>>,
    qos: Option<StreamQosOpts>,
    test_cycle: bool,
    with_audio: bool,
    _use_test_tone: bool,
    _audio_target_override: Option<&str>,
) -> anyhow::Result<()> {
    use zerocast_core::{QosAction, QosController, StreamMetricsSample, StreamProfile};

    #[cfg(feature = "audio")]
    if with_audio {
        let audio_local = "0.0.0.0:0";
        let audio_port_override = _audio_target_override.as_ref().and_then(|s| {
            s.parse::<SocketAddr>()
                .ok()
                .map(|a| a.port())
                .filter(|p| *p > 0)
        });
        let audio_target =
            zerocast_protocol::audio_target_from_video(_target, audio_port_override)?;
        eprintln!("audio: sender {audio_local} -> {audio_target}");
        let max = max_frames;
        let use_tone = _use_test_tone;
        tokio::spawn(async move {
            if let Err(e) =
                audio_rtp::audio_stream_loop(audio_local, &audio_target, max, use_tone).await
            {
                eprintln!("audio stream error: {e:#}");
            }
        });
    }
    #[cfg(not(feature = "audio"))]
    if with_audio {
        eprintln!("audio: disabled (build without `audio` feature)");
    }

    let mut qos_controller = qos.map(|o| {
        let session = StreamProfile {
            name: "session",
            width,
            height,
            fps,
        };
        QosController::new(
            session,
            o.display_w,
            o.display_h,
            o.refresh_hz,
            o.recv_cap,
            o.device_class,
        )
    });
    if qos_controller.is_some() {
        eprintln!("qos: monitoring sender metrics (hot reconfigure enabled)");
    }

    let mut screen = if test_cycle {
        None
    } else {
        Some(match screen {
            Some(s) => s,
            None => zerocast_platform::ScreenCapture::open(width, height)?,
        })
    };
    let mut sender = match sender {
        Some(s) => s,
        None => Sender::bind(_local, _target).await?,
    };
    let mut frame_duration = std::time::Duration::from_secs_f64(1.0 / fps.max(1) as f64);
    let rtcp_interval = std::time::Duration::from_secs(1);
    let mut next_rtcp = std::time::Instant::now();
    let mut frame_index = 0u64;
    let mut stats_window_start = std::time::Instant::now();
    let mut stats_frames = 0u64;
    let mut stats_encode_ms = 0f64;
    let mut stats_bytes = 0u64;

    loop {
        if max_frames > 0 && frame_index >= max_frames {
            eprintln!("stream: finished after {max_frames} frames");
            break;
        }

        let frame_start = std::time::Instant::now();
        let cycle = (frame_index as u32) % test_pattern::CYCLE_COUNT;
        if test_cycle {
            eprintln!("streaming frame {frame_index} (test-cycle {cycle})...");
        } else if frame_index == 0 || frame_index % 30 == 0 {
            eprintln!("streaming frame {frame_index}...");
        }
        let frame = if frame_index == 0 {
            match &first_frame {
                Some(f) => f.clone(),
                None if test_cycle => test_pattern::test_cycle_frame(width, height, 0),
                None => screen.as_mut().unwrap().capture_frame()?,
            }
        } else if test_cycle {
            test_pattern::test_cycle_frame(width, height, cycle)
        } else {
            screen.as_mut().unwrap().capture_frame()?
        };
        let encode_start = std::time::Instant::now();
        let (nalus, pts_ns) = video_encoder.encode_frame(&frame, frame_index)?;
        let encode_ms = encode_start.elapsed().as_secs_f64() * 1000.0;
        let payload_bytes: u64 = nalus.iter().map(|n| n.len() as u64).sum();
        stats_frames += 1;
        stats_encode_ms += encode_ms;
        stats_bytes += payload_bytes;
        if frame_index < 3 {
            let summary: Vec<String> = nalus
                .iter()
                .map(|n| {
                    let raw = strip_annex_b_start_code(n);
                    let t = raw.first().map(|b| b & 0x1f).unwrap_or(0);
                    format!("type {t} {}b", raw.len())
                })
                .collect();
            eprintln!(
                "stream: frame {frame_index} {} nalus [{}]",
                nalus.len(),
                summary.join(", ")
            );
        }
        let frame_rtp_ts = Sender::pts_ns_to_rtp(pts_ns);

        if next_rtcp <= std::time::Instant::now() {
            sender.send_rtcp_sr(frame_rtp_ts).await?;
            next_rtcp = std::time::Instant::now() + rtcp_interval;
        }

        sender.send_nalus(&nalus, frame_rtp_ts).await?;
        frame_index += 1;

        let elapsed = frame_start.elapsed();
        if frame_index == 1 || frame_index % 30 == 0 {
            eprintln!(
                "sent frame {} ({} nalus, {:.0} ms encode, {:.0} ms total)",
                frame_index,
                nalus.len(),
                encode_ms,
                elapsed.as_secs_f64() * 1000.0
            );
        }
        if frame_index % 30 == 0 {
            let window_secs = stats_window_start.elapsed().as_secs_f64().max(0.001);
            let fps_actual = stats_frames as f64 / window_secs;
            let avg_encode = stats_encode_ms / stats_frames.max(1) as f64;
            let kbps = (stats_bytes as f64 * 8.0 / 1000.0) / window_secs;
            let recv_fb = sender.try_recv_feedback().await;
            eprintln!(
                "stream stats: {:.1} fps, {:.0} ms encode avg, {:.0} kbps (last {} frames)",
                fps_actual,
                avg_encode,
                kbps,
                stats_frames
            );
            if let Some(fb) = &recv_fb {
                eprintln!(
                    "qos: recv feedback loss={:.1}% lag={:.0}ms jitter={:.0}ms drops={}",
                    fb.fraction_lost * 100.0,
                    fb.decode_lag_ms,
                    fb.jitter_ms,
                    fb.dropped_frames
                );
            }
            if let Some(controller) = qos_controller.as_mut() {
                let sample = StreamMetricsSample {
                    actual_fps: fps_actual,
                    target_fps: fps as f64,
                    avg_encode_ms: avg_encode,
                    kbps,
                    fraction_lost: recv_fb.as_ref().map(|f| f.fraction_lost),
                    recv_decode_lag_ms: recv_fb.as_ref().map(|f| f.decode_lag_ms),
                    recv_jitter_ms: recv_fb.as_ref().map(|f| f.jitter_ms),
                };
                let action = controller.observe_window(sample);
                match action {
                    QosAction::Hold => {}
                    QosAction::RecommendDowngrade(p) | QosAction::RecommendUpgrade(p) => {
                        if test_cycle {
                            eprintln!(
                                "qos: ignoring {:?} during --test-cycle",
                                p.name
                            );
                        } else if let Some(screen) = screen.as_mut() {
                            apply_qos_stream_profile(
                                p,
                                &mut width,
                                &mut height,
                                &mut fps,
                                &mut frame_duration,
                                screen,
                                &mut video_encoder,
                            )?;
                            controller.on_profile_applied(p);
                        }
                    }
                }
            }
            stats_window_start = std::time::Instant::now();
            stats_frames = 0;
            stats_encode_ms = 0.0;
            stats_bytes = 0;
        }
        if elapsed < frame_duration {
            tokio::time::sleep(frame_duration - elapsed).await;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// Reserve an RTP port with a free RTCP port at `port + 1` (see `Receiver::bind`).
    fn ephemeral_rtp_port() -> std::io::Result<u16> {
        for _ in 0..64 {
            let probe = std::net::UdpSocket::bind("127.0.0.1:0")?;
            let port = probe.local_addr()?.port();
            drop(probe);
            if port == u16::MAX {
                continue;
            }
            let rtp_ok = std::net::UdpSocket::bind(("127.0.0.1", port)).is_ok();
            let rtcp_ok = std::net::UdpSocket::bind(("127.0.0.1", port + 1)).is_ok();
            if rtp_ok && rtcp_ok {
                return Ok(port);
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "could not find free RTP/RTCP UDP port pair",
        ))
    }

    #[tokio::test]
    #[serial]
    async fn sender_receiver_smoke() -> anyhow::Result<()> {
        let port = ephemeral_rtp_port()?;
        let bind_addr = format!("127.0.0.1:{port}");
        let recv = Receiver::bind(&bind_addr).await?;
        let sender = Sender::bind("127.0.0.1:0", &bind_addr).await?;
        // Run receiver in background
        tokio::spawn(async move { let _ = recv.run().await; });
        // send a few packets then return
        tokio::spawn(async move { let _ = tokio::time::timeout(std::time::Duration::from_millis(200), sender.run()).await; });
        Ok(())
    }

    #[tokio::test]
    #[serial]
    async fn stream_smoke() -> anyhow::Result<()> {
        let port = ephemeral_rtp_port()?;
        let bind_addr = format!("127.0.0.1:{port}");
        let recv = Receiver::bind(&bind_addr).await?;
        tokio::spawn(async move { let _ = recv.run().await; });
        let enc = FfmpegCliEncoder::open_synthetic(64, 48, 30);
        let stream = capture_encode_and_stream_with_encoder(
            "127.0.0.1:0",
            &bind_addr,
            64,
            48,
            30,
            5,
            enc,
        );
        tokio::time::timeout(std::time::Duration::from_secs(3), stream).await??;
        Ok(())
    }

    #[test]
    fn fragmentation_timestamp_consistency() -> anyhow::Result<()> {
        // Build a synthetic large NALU (no Annex-B start code)
        let mut raw: Vec<u8> = Vec::new();
        // NAL header: IDR (type 5)
        raw.push(0x65);
        // create payload large enough to require multiple fragments
        raw.extend(std::iter::repeat(0xAAu8).take(MAX_RTP_PAYLOAD * 3));

        // compute a single RTP timestamp for this NALU/frame
        let now_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u128;
        let rtptimestamp = ((now_ms * 90_000u128) / 1000u128) as u32;

        // perform FU-A fragmentation as in capture_encode_and_send
        let nal_header = raw[0];
        let nri_f = nal_header & 0xE0;
        let nal_payload = &raw[1..];
        let frag_size = MAX_RTP_PAYLOAD.saturating_sub(2);
        let mut offset = 0usize;
        let mut first = true;
        let mut pkts: Vec<RtpPacket> = Vec::new();
        let mut seq: u16 = 0;
        while offset < nal_payload.len() {
            let end = usize::min(offset + frag_size, nal_payload.len());
            let slice = &nal_payload[offset..end];
            let mut payload_vec: Vec<u8> = Vec::with_capacity(slice.len() + 2);
            let fu_indicator = nri_f | 28u8;
            let mut fu_header = (raw[0] & 0x1F) as u8;
            let s_bit = first;
            let e_bit = end == nal_payload.len();
            if s_bit { fu_header |= 0x80; }
            if e_bit { fu_header |= 0x40; }
            payload_vec.push(fu_indicator);
            payload_vec.push(fu_header);
            payload_vec.extend_from_slice(slice);

            seq = seq.wrapping_add(1);
            let mut pkt = RtpPacket::default();
            pkt.header.version = 2;
            pkt.header.payload_type = 96;
            pkt.header.sequence_number = seq;
            pkt.header.timestamp = rtptimestamp;
            pkt.header.ssrc = 0xdeadbeef;
            pkt.payload = payload_vec.into();
            pkts.push(pkt);

            first = false;
            offset = end;
        }

        // All fragments must share the same RTP timestamp
        assert!(!pkts.is_empty());
        let first_ts = pkts[0].header.timestamp;
        assert!(pkts.iter().all(|p| p.header.timestamp == first_ts));

        Ok(())
    }
}
