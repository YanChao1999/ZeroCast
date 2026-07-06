//! Opus audio RTP sender/receiver (spec §5).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;

use rtp::packet::Packet as RtpPacket;
use webrtc_util::marshal::Marshal;
use webrtc_util::Unmarshal;
use zerocast_protocol::rtp::PT_OPUS;

use crate::AvSyncState;

pub struct AudioSender {
    socket: tokio::net::UdpSocket,
    target: SocketAddr,
    seq: u16,
    ssrc: u32,
}

impl AudioSender {
    pub async fn bind(local: &str, target: &str) -> anyhow::Result<Self> {
        let socket = tokio::net::UdpSocket::bind(local).await?;
        let target: SocketAddr = target.parse()?;
        let ssrc = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u32 ^ 0xA000_0000;
        Ok(Self {
            socket,
            target,
            seq: 0,
            ssrc,
        })
    }

    fn rtcp_target(&self) -> Option<SocketAddr> {
        zerocast_protocol::rtcp_port(self.target.port())
            .map(|p| SocketAddr::new(self.target.ip(), p))
    }

    pub async fn send_rtcp_sr(&self, rtp_ts: u32) -> anyhow::Result<()> {
        let Some(rtcp_target) = self.rtcp_target() else {
            return Ok(());
        };
        const NTP_UNIX_OFFSET: u64 = 2_208_988_800u64;
        let now_dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let unix_secs = now_dur.as_secs();
        let unix_nanos = now_dur.subsec_nanos() as u128;
        let ntp_secs = unix_secs.saturating_add(NTP_UNIX_OFFSET) as u32;
        let ntp_frac = ((unix_nanos * (1u128 << 32)) / 1_000_000_000u128) as u32;

        let length: u16 = 6;
        let mut sr: Vec<u8> = Vec::with_capacity(28);
        sr.push(0x80u8);
        sr.push(200u8);
        sr.push(((length >> 8) & 0xFF) as u8);
        sr.push((length & 0xFF) as u8);
        sr.extend_from_slice(&self.ssrc.to_be_bytes());
        sr.extend_from_slice(&ntp_secs.to_be_bytes());
        sr.extend_from_slice(&ntp_frac.to_be_bytes());
        sr.extend_from_slice(&rtp_ts.to_be_bytes());
        sr.extend_from_slice(&0u32.to_be_bytes());
        sr.extend_from_slice(&0u32.to_be_bytes());

        let _ = self.socket.send_to(&sr, &rtcp_target).await;
        Ok(())
    }

    pub async fn send_opus(&mut self, payload: &[u8], rtp_ts: u32) -> anyhow::Result<()> {
        self.seq = self.seq.wrapping_add(1);
        let mut pkt = RtpPacket::default();
        pkt.header.version = 2;
        pkt.header.payload_type = PT_OPUS;
        pkt.header.sequence_number = self.seq;
        pkt.header.timestamp = rtp_ts;
        pkt.header.ssrc = self.ssrc;
        pkt.header.marker = false;
        pkt.payload = payload.to_vec().into();
        let buf = pkt.marshal()?;
        let _ = self.socket.send_to(&buf, &self.target).await?;
        Ok(())
    }
}

pub struct AudioReceiver {
    socket: tokio::net::UdpSocket,
}

impl AudioReceiver {
    pub async fn bind(local: &str) -> anyhow::Result<Self> {
        let socket = tokio::net::UdpSocket::bind(local).await?;
        Ok(Self { socket })
    }

    pub async fn run_log(self, sync: Option<Arc<AvSyncState>>) -> anyhow::Result<()> {
        let mut buf = vec![0u8; 1500];
        loop {
            let (n, _addr) = self.socket.recv_from(&mut buf).await?;
            if n < 12 {
                continue;
            }
            match RtpPacket::unmarshal(&mut &buf[..n]) {
                Ok(pkt) if pkt.header.payload_type == PT_OPUS => {
                    if let Some(s) = &sync {
                        s.on_audio_rtp_ts(pkt.header.timestamp);
                    }
                    println!("audio rtp bytes={}", pkt.payload.len());
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("audio: failed to parse RTP: {e}");
                }
            }
        }
    }

    pub async fn run_playback(self, sync: Option<Arc<AvSyncState>>) -> anyhow::Result<()> {
        #[cfg(not(feature = "audio-playback"))]
        {
            let _ = sync;
            anyhow::bail!("audio playback requires the `audio-playback` feature");
        }

        #[cfg(feature = "audio-playback")]
        {
            use zerocast_audio::{OpusDecoder, playback::cpal_output::CpalPlayback};
            use zerocast_protocol::audio::CHANNELS;

            let (pcm_tx, pcm_rx) = std::sync::mpsc::sync_channel::<Vec<i16>>(16);
            let _play_thread = std::thread::spawn(move || -> anyhow::Result<()> {
                let (playback, _stream) = CpalPlayback::open_default(CHANNELS)?;
                eprintln!("audio: cpal playback open ({} ch)", playback.channels());
                while let Ok(samples) = pcm_rx.recv() {
                    playback.push_pcm(&samples);
                }
                Ok(())
            });

            let decoder = OpusDecoder::new_stereo()?;
            let mut buf = vec![0u8; 1500];
            let mut packets = 0u64;
            loop {
                let (n, _addr) = self.socket.recv_from(&mut buf).await?;
                if n < 12 {
                    continue;
                }
                match RtpPacket::unmarshal(&mut &buf[..n]) {
                    Ok(pkt) if pkt.header.payload_type == PT_OPUS => {
                        if let Some(s) = &sync {
                            s.on_audio_rtp_ts(pkt.header.timestamp);
                        }
                        match decoder.decode(&pkt.payload) {
                            Ok(pcm) => {
                                packets += 1;
                                if packets == 1 {
                                    eprintln!(
                                        "audio: first Opus frame decoded ({} samples, playback)",
                                        pcm.samples.len()
                                    );
                                }
                                if pcm_tx.try_send(pcm.samples).is_err() {
                                    eprintln!("audio: playback queue full, dropping frame");
                                }
                            }
                            Err(e) if packets < 5 => {
                                eprintln!("audio: decode error: {e:#}");
                            }
                            Err(_) => {}
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("audio: failed to parse RTP: {e}");
                    }
                }
            }
        }
    }
}

enum AudioPcmSource {
    Sine(zerocast_audio::SineSource),
    #[cfg(feature = "audio-capture")]
    Mic {
        rx: tokio::sync::mpsc::UnboundedReceiver<zerocast_audio::PcmFrame>,
        _thread: std::thread::JoinHandle<()>,
    },
}

impl AudioPcmSource {
    async fn next_pcm_frame(&mut self) -> anyhow::Result<zerocast_audio::PcmFrame> {
        match self {
            AudioPcmSource::Sine(s) => zerocast_audio::TestToneSource::next_pcm_frame(s),
            #[cfg(feature = "audio-capture")]
            AudioPcmSource::Mic { rx, .. } => rx
                .recv()
                .await
                .ok_or_else(|| anyhow::anyhow!("microphone capture thread stopped")),
        }
    }
}

#[cfg(feature = "audio-capture")]
fn spawn_mic_pcm_source() -> anyhow::Result<AudioPcmSource> {
    use zerocast_audio::{CpalMicSource, TestToneSource};

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = std::thread::spawn(move || {
        let Ok((mut source, _stream)) = CpalMicSource::open_default() else {
            return;
        };
        eprintln!("audio: using default microphone input");
        loop {
            match TestToneSource::next_pcm_frame(&mut source) {
                Ok(frame) => {
                    if tx.send(frame).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("audio: mic error: {e:#}");
                    break;
                }
            }
        }
    });
    Ok(AudioPcmSource::Mic {
        rx,
        _thread: handle,
    })
}

fn open_pcm_source(use_test_tone: bool) -> anyhow::Result<AudioPcmSource> {
    if use_test_tone {
        return Ok(AudioPcmSource::Sine(zerocast_audio::SineSource::new(440.0)));
    }
    #[cfg(feature = "audio-capture")]
    {
        match spawn_mic_pcm_source() {
            Ok(source) => return Ok(source),
            Err(e) => {
                eprintln!("audio: mic unavailable ({e:#}), falling back to test tone");
            }
        }
    }
    #[cfg(not(feature = "audio-capture"))]
    if !use_test_tone {
        eprintln!("audio: capture not enabled; using 440 Hz test tone (pass --test-tone explicitly)");
    }
    Ok(AudioPcmSource::Sine(zerocast_audio::SineSource::new(440.0)))
}

/// Opus audio stream (20 ms frames) for `--audio`.
pub async fn audio_stream_loop(
    local: &str,
    target: &str,
    max_frames: u64,
    use_test_tone: bool,
) -> anyhow::Result<()> {
    use zerocast_audio::OpusEncoder;

    let mut sender = AudioSender::bind(local, target)
        .await
        .with_context(|| format!("audio RTP bind {local}"))?;
    let mut source = open_pcm_source(use_test_tone)?;
    let encoder = OpusEncoder::new_stereo()?;
    let frame_duration = Duration::from_millis(zerocast_protocol::rtp::AUDIO_FRAME_MS as u64);
    let rtcp_interval = Duration::from_secs(1);
    let mut next_rtcp = std::time::Instant::now();
    let mut rtp_ts: u32 = 0;
    let mut frame_index = 0u64;

    eprintln!(
        "audio: streaming Opus to {target} ({} Hz, {} ms frames)",
        zerocast_protocol::rtp::AUDIO_CLOCK_HZ,
        zerocast_protocol::rtp::AUDIO_FRAME_MS
    );

    loop {
        if max_frames > 0 && frame_index >= max_frames {
            eprintln!("audio: finished after {max_frames} frames");
            break;
        }
        let pcm = source.next_pcm_frame().await?;
        let opus = encoder
            .encode(&pcm)
            .with_context(|| "Opus encode (ffmpeg libopus)")?;
        if next_rtcp <= std::time::Instant::now() {
            sender.send_rtcp_sr(rtp_ts).await?;
            next_rtcp = std::time::Instant::now() + rtcp_interval;
        }
        sender.send_opus(&opus, rtp_ts).await?;
        rtp_ts = rtp_ts.wrapping_add(zerocast_protocol::rtp::AUDIO_SAMPLES_PER_FRAME);
        frame_index += 1;
        tokio::time::sleep(frame_duration).await;
    }
    Ok(())
}

/// Options for combined video/audio receive paths.
#[derive(Debug, Clone, Copy)]
pub struct RecvAvOpts {
    pub with_audio: bool,
    /// Decode Opus and play via cpal (requires `audio-playback`).
    pub playback: bool,
    pub video_fps: u32,
}

impl RecvAvOpts {
    pub fn log_only(with_audio: bool, video_fps: u32) -> Self {
        Self {
            with_audio,
            playback: false,
            video_fps,
        }
    }
}

/// Run video recv-log and optional audio recv (log or playback).
pub async fn recv_log_av(video_local: &str, opts: RecvAvOpts) -> anyhow::Result<()> {
    if !opts.with_audio {
        return Receiver::bind(video_local).await?.run().await;
    }
    let audio_local = zerocast_protocol::audio_bind_from_video(video_local, None)?;
    eprintln!(
        "audio: listening on {audio_local}{}",
        if opts.playback { " (playback)" } else { "" }
    );
    let sync = Arc::new(AvSyncState::new(opts.video_fps));
    let video = Receiver::bind(video_local).await?;
    let audio = AudioReceiver::bind(&audio_local).await?;
    let sync_v = sync.clone();
    let sync_a = sync.clone();
    let playback = opts.playback;
    tokio::try_join!(
        video.run_frame_delivery(move |_ssrc, _ts, au| {
            sync_v.on_video_frame();
            println!("rtp access unit bytes={}", au.len());
        }),
        async move {
            if playback {
                audio.run_playback(Some(sync_a)).await
            } else {
                audio.run_log(Some(sync_a)).await
            }
        }
    )?;
    Ok(())
}

use super::Receiver;
