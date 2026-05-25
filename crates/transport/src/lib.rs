use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::HashMap;

use rtp::packet::Packet as RtpPacket;
// webrtc-util marshal/unmarshal traits used by the `rtp` crate
use webrtc_util::marshal::{Marshal, Unmarshal};
mod capture;
mod encoder;

pub struct Sender {
    socket: tokio::net::UdpSocket,
    target: SocketAddr,
    seq: u16,
    ssrc: u32,
    payload_type: u8,
}

pub struct Receiver {
    socket: tokio::net::UdpSocket,
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
}

impl Receiver {
    pub async fn bind(local: &str) -> anyhow::Result<Self> {
        let socket = tokio::net::UdpSocket::bind(local).await?;
        Ok(Self { socket })
    }

    /// Run receiver loop parsing minimal RTP header and printing basic stats.
    pub async fn run(self) -> anyhow::Result<()> {
        let mut buf = vec![0u8; 1500];
        let mut last_seq: Option<u16> = None;
        // per-ssrc mapping: base RTP timestamp -> wall-clock ns at first receipt
        let mut ssrc_bases: HashMap<u32, (u32, u128)> = HashMap::new();
        // per-ssrc last RTP timestamp to compute per-packet expected intervals
        let mut ssrc_last_ts: HashMap<u32, u32> = HashMap::new();
        // previous arrival time (ns) for inter-arrival measurement
        let mut prev_now_ns: Option<u128> = None;
        loop {
            let (n, addr) = self.socket.recv_from(&mut buf).await?;
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
                    let payload = &pkt.payload;
                    // Estimate latency per-SSRC by anchoring first seen RTP timestamp to wall-clock
                    const RTP_CLOCK_RATE: u32 = 90_000;
                    let now_ns = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() as u128;
                    let latency_ms: u64 = match ssrc_bases.get(&ssrc) {
                        None => {
                            // first packet from this SSRC: record base mapping and show 0 latency
                            ssrc_bases.insert(ssrc, (ts, now_ns));
                            0
                        }
                        Some((base_ts, base_now_ns)) => {
                            // if timestamp moved backwards (sender restarted or wrapped), re-anchor
                            if ts < *base_ts {
                                ssrc_bases.insert(ssrc, (ts, now_ns));
                                ssrc_last_ts.insert(ssrc, ts);
                                prev_now_ns = Some(now_ns);
                                0
                            } else {
                                let delta_ticks = ts.wrapping_sub(*base_ts) as u128;
                                // expected send time in ns: base_now_ns + delta_ticks * (1e9 / RTP_CLOCK_RATE)
                                let expected_send_ns = base_now_ns.saturating_add(delta_ticks.saturating_mul(1_000_000_000u128) / (RTP_CLOCK_RATE as u128));
                                // compute per-packet expected interval using last seen RTP timestamp
                                let expected_interval_ms = match ssrc_last_ts.get(&ssrc) {
                                    Some(prev_ts) => {
                                        let delta_since_last = ts.wrapping_sub(*prev_ts) as u128;
                                        (delta_since_last as f64) * 1000.0 / (RTP_CLOCK_RATE as f64)
                                    }
                                    None => 0.0,
                                };
                                // compute actual arrival interval
                                let arrival_interval_ms = match prev_now_ns {
                                    Some(pn) => ((now_ns - pn) as f64) / 1_000_000.0,
                                    None => 0.0,
                                };
                                // update last_ts and prev_now_ns
                                ssrc_last_ts.insert(ssrc, ts);
                                prev_now_ns = Some(now_ns);
                                if now_ns > expected_send_ns {
                                    ((now_ns - expected_send_ns) / 1_000_000u128) as u64
                                } else {
                                    0
                                }
                            }
                        }
                    };
                    let lost = match last_seq {
                        Some(prev) if seq > prev && seq - prev > 1 => (seq - prev - 1) as u32,
                        _ => 0,
                    };
                    last_seq = Some(seq);
                    println!("rtp pkt seq={} pt={} ssrc=0x{:08x} payload={} latency_approx={}ms lost_since_last={}", seq, pt, ssrc, payload.len(), latency_ms, lost);
                }
                Err(e) => {
                    eprintln!("failed to parse RTP packet from {}: {}", addr, e);
                    continue;
                }
            }
        }
    }
}

/// Capture one synthetic frame, encode to Annex-B H.264 NALUs (stub), and send each NALU as
/// an RTP packet (one NALU per RTP packet — placeholder behavior until proper FU-A fragmentation is added).
pub async fn capture_encode_and_send(local: &str, target: &str) -> anyhow::Result<()> {
    // basic parameters
    let width = 640u32;
    let height = 360u32;
    // bind sender
    let mut sender = Sender::bind(local, target).await?;
    // capture a single frame (frame_index = 1)
    let frame = capture::capture_frame(width, height, 1)?;
    // encode to Annex-B NALUs (fake encoder for now)
    let nalus = encoder::encode_frame_to_h264_annexb(&frame, width, height, 1)?;
    // send each NALU as an RTP packet payload
    for nalu in nalus {
        sender.seq = sender.seq.wrapping_add(1);
        let mut pkt = RtpPacket::default();
        pkt.header.version = 2;
        pkt.header.payload_type = sender.payload_type;
        pkt.header.sequence_number = sender.seq;
        pkt.header.timestamp = sender.ssrc; // placeholder timestamp
        pkt.header.ssrc = sender.ssrc;
        // use the entire Annex-B NALU as payload for now
        pkt.payload = nalu.into();
        let buf = pkt.marshal()?;
        let _ = sender.socket.send_to(&buf, &sender.target).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sender_receiver_smoke() -> anyhow::Result<()> {
        let recv = Receiver::bind("127.0.0.1:50010").await?;
        let sender = Sender::bind("127.0.0.1:0", "127.0.0.1:50010").await?;
        // Run receiver in background
        tokio::spawn(async move { let _ = recv.run().await; });
        // send a few packets then return
        tokio::spawn(async move { let _ = tokio::time::timeout(std::time::Duration::from_millis(200), sender.run()).await; });
        Ok(())
    }
}
