//! Opus audio RTP sender/receiver (spec §5).

use std::net::SocketAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rtp::packet::Packet as RtpPacket;
use webrtc_util::marshal::Marshal;
use webrtc_util::Unmarshal;
use zerocast_protocol::rtp::PT_OPUS;

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

    pub async fn run_log(self) -> anyhow::Result<()> {
        let mut buf = vec![0u8; 1500];
        loop {
            let (n, _addr) = self.socket.recv_from(&mut buf).await?;
            if n < 12 {
                continue;
            }
            match RtpPacket::unmarshal(&mut &buf[..n]) {
                Ok(pkt) if pkt.header.payload_type == PT_OPUS => {
                    println!("audio rtp bytes={}", pkt.payload.len());
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("audio: failed to parse RTP: {e}");
                }
            }
        }
    }
}

/// Opus sine-tone stream (20 ms frames) for `--audio` smoke tests.
pub async fn audio_stream_loop(
    local: &str,
    target: &str,
    max_frames: u64,
) -> anyhow::Result<()> {
    use zerocast_audio::{OpusEncoder, SineSource, TestToneSource};

    let mut sender = AudioSender::bind(local, target).await?;
    let mut tone = SineSource::new(440.0);
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
        let pcm = tone.next_pcm_frame()?;
        let opus = encoder.encode(&pcm)?;
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

/// Run video recv-log and optional audio recv-log concurrently.
pub async fn recv_log_av(video_local: &str, with_audio: bool) -> anyhow::Result<()> {
    if !with_audio {
        return Receiver::bind(video_local).await?.run().await;
    }
    let audio_local = zerocast_protocol::audio_bind_from_video(video_local, None)?;
    eprintln!("audio: listening on {audio_local}");
    let video = Receiver::bind(video_local).await?;
    let audio = AudioReceiver::bind(&audio_local).await?;
    tokio::try_join!(video.run(), audio.run_log())?;
    Ok(())
}

use super::Receiver;
