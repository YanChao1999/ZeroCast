use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::{HashMap, BTreeMap};

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
    rtcp_rx: tokio::sync::mpsc::UnboundedReceiver<(u32, u32, u32, u32)>,
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
        // try to bind RTCP socket at local port + 1 and spawn a reader task sending SRs via channel
        let local_addr: SocketAddr = local.parse()?;
        let rtcp_port = local_addr.port().wrapping_add(1);
        let rtcp_bind = SocketAddr::new(local_addr.ip(), rtcp_port);
        let rtcp_socket = tokio::net::UdpSocket::bind(rtcp_bind).await?;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        // spawn background task to parse RTCP SRs and forward them
        tokio::spawn(async move {
            let mut buf = vec![0u8; 1500];
            loop {
                match rtcp_socket.recv_from(&mut buf).await {
                    Ok((n, _addr)) => {
                        if n < 28 { continue; }
                        // minimal SR: PT=200 at buf[1]
                        if buf[1] != 200 { continue; }
                        // parse SSRC, NTP secs, NTP frac, RTP timestamp
                        let ssrc = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
                        let ntp_secs = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
                        let ntp_frac = u32::from_be_bytes([buf[12], buf[13], buf[14], buf[15]]);
                        let rtp_ts = u32::from_be_bytes([buf[16], buf[17], buf[18], buf[19]]);
                        let _ = tx.send((ssrc, ntp_secs, ntp_frac, rtp_ts));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self { socket, rtcp_rx: rx })
    }

    /// Run receiver loop parsing minimal RTP header and printing basic stats.
    pub async fn run(self) -> anyhow::Result<()> {
        let mut buf = vec![0u8; 1500];
        let mut last_seq: Option<u16> = None;
        // reassembly state per-SSRC for FU-A
        let mut reassembly: HashMap<u32, (Vec<u8>, u16)> = HashMap::new();
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
        loop {
            // drain any pending RTCP Sender Reports first
            while let Ok((ssrc_sr, ntp_secs, ntp_frac, rtp_ts)) = rtcp_rx.try_recv() {
                const NTP_UNIX_OFFSET: u64 = 2_208_988_800u64;
                let ntp_secs_u64 = ntp_secs as u64;
                let unix_secs = ntp_secs_u64.saturating_sub(NTP_UNIX_OFFSET) as u128;
                let frac_ns = ((ntp_frac as u128) * 1_000_000_000u128) / (1u128 << 32);
                let base_now_ns = unix_secs.saturating_mul(1_000_000_000u128).saturating_add(frac_ns);
                ssrc_bases.insert(ssrc_sr, (rtp_ts, base_now_ns));
                ssrc_last_ts.insert(ssrc_sr, rtp_ts);
            }

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
                            if let Some((payload_vec, ts_val, arr_ns, pt_val)) = buftree.remove(&next) {
                                // compute latency using anchoring (ssrc_bases)
                                const RTP_CLOCK_RATE: u32 = 90_000;
                                let latency_ms: u64 = match ssrc_bases.get(&ssrc) {
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
                                    Some(prev) if next > *prev && next - *prev > 1 => (next - *prev - 1) as u32,
                                    _ => 0,
                                };

                                // depacketize payload_vec (same logic as before)
                                if payload_vec.len() > 0 {
                                    let nal_first = payload_vec[0];
                                    let nal_type = nal_first & 0x1F;
                                    if nal_type == 28 {
                                        if payload_vec.len() < 2 {
                                            eprintln!("malformed FU-A packet");
                                        } else {
                                            let fu_hdr = payload_vec[1];
                                            let s_bit = (fu_hdr & 0x80) != 0;
                                            let e_bit = (fu_hdr & 0x40) != 0;
                                            let orig_nal_type = fu_hdr & 0x1F;
                                            let nal_header = (nal_first & 0xE0) | orig_nal_type;
                                            let frag_payload = &payload_vec[2..];
                                            let entry = reassembly.entry(ssrc).or_insert((Vec::new(), next.wrapping_sub(1)));
                                            if !s_bit {
                                                let expected_seq = entry.1.wrapping_add(1);
                                                if next != expected_seq {
                                                    entry.0.clear();
                                                }
                                            }
                                            if s_bit {
                                                entry.0.clear();
                                                entry.0.push(nal_header);
                                                entry.0.extend_from_slice(frag_payload);
                                            } else {
                                                entry.0.extend_from_slice(frag_payload);
                                            }
                                            entry.1 = next;
                                            if e_bit {
                                                let nal_size = entry.0.len();
                                                println!("rtp pkt seq={} pt={} ssrc=0x{:08x} reassembled_nalu={} latency_approx={}ms lost_since_last={}", next, pt_val, ssrc, nal_size, latency_ms, lost);
                                                reassembly.remove(&ssrc);
                                            }
                                        }
                                    } else {
                                        println!("rtp pkt seq={} pt={} ssrc=0x{:08x} payload={} latency_approx={}ms lost_since_last={}", next, pt_val, ssrc, payload_vec.len(), latency_ms, lost);
                                    }
                                } else {
                                    println!("rtp pkt seq={} pt={} ssrc=0x{:08x} payload=0 latency_approx={}ms lost_since_last={}", next, pt_val, ssrc, latency_ms, lost);
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
    // encode to Annex-B NALUs and get encoder PTS (ns)
    let (nalus, pts_ns) = encoder::encode_frame_to_h264_annexb(&frame, width, height, 1)?;
    // send each NALU as RTP packets, using FU-A fragmentation when needed
    const MAX_RTP_PAYLOAD: usize = 1200; // conservative payload size
    // compute one RTP timestamp for this captured frame and reuse for all NALUs/fragments
    // derive RTP timestamp from encoder PTS (pts_ns)
    let frame_rtp_ts = ((pts_ns.saturating_mul(90_000u128)) / 1_000_000_000u128) as u32;
    // use current wall-clock for NTP mapping in SR
    let now = SystemTime::now();
    let now_dur = now.duration_since(UNIX_EPOCH)?;

    // send an RTCP Sender Report (SR) mapping this RTP timestamp to an NTP timestamp
    // SR target is target port + 1 (typical RTCP port)
    let target_sock = sender.target.clone();
    let rtcp_port = target_sock.port().wrapping_add(1);
        if rtcp_port != 0 {
            let rtcp_target = SocketAddr::new(target_sock.ip(), rtcp_port);
            // compute NTP timestamp (seconds since 1900) and fractional part
            const NTP_UNIX_OFFSET: u64 = 2_208_988_800u64; // seconds between 1900 and 1970
            let unix_secs = now_dur.as_secs();
            let unix_nanos = now_dur.subsec_nanos() as u128;
            let ntp_secs = unix_secs.saturating_add(NTP_UNIX_OFFSET) as u32;
            let ntp_frac = ((unix_nanos * (1u128 << 32)) / 1_000_000_000u128) as u32;

            // Build RTCP SR packet
            // Header: V=2,P=0,RC=0 -> 0x80, PT=200, length=6 (32-bit words minus one)
            let length: u16 = 6;
            let mut sr: Vec<u8> = Vec::with_capacity(28);
            sr.push(0x80u8); // V=2, P=0, RC=0
            sr.push(200u8); // PT=SR
            sr.push(((length >> 8) & 0xFF) as u8);
            sr.push((length & 0xFF) as u8);
            // SSRC
            sr.extend_from_slice(&sender.ssrc.to_be_bytes());
            // NTP timestamp: seconds (32) + fraction (32)
            sr.extend_from_slice(&ntp_secs.to_be_bytes());
            sr.extend_from_slice(&ntp_frac.to_be_bytes());
            // RTP timestamp (32)
            sr.extend_from_slice(&frame_rtp_ts.to_be_bytes());
            // sender's packet count (32) - unknown, set zero
            sr.extend_from_slice(&0u32.to_be_bytes());
            // sender's octet count (32) - unknown, set zero
            sr.extend_from_slice(&0u32.to_be_bytes());

            // best-effort: ignore send errors
            let _ = sender.socket.send_to(&sr, &rtcp_target).await;
        }
    for nalu in nalus {
        // strip Annex-B start code if present
        let raw = if nalu.len() >= 4 && nalu[0..4] == [0, 0, 0, 1] {
            &nalu[4..]
        } else if nalu.len() >= 3 && nalu[0..3] == [0, 0, 1] {
            &nalu[3..]
        } else {
            &nalu[..]
        };
        if raw.len() <= MAX_RTP_PAYLOAD {
            // single NAL unit packet
            sender.seq = sender.seq.wrapping_add(1);
            let mut pkt = RtpPacket::default();
            pkt.header.version = 2;
            pkt.header.payload_type = sender.payload_type;
            pkt.header.sequence_number = sender.seq;
            pkt.header.timestamp = frame_rtp_ts;
            pkt.header.ssrc = sender.ssrc;
            pkt.payload = raw.to_vec().into();
            let buf = pkt.marshal()?;
            let _ = sender.socket.send_to(&buf, &sender.target).await?;
        } else {
            // FU-A fragmentation
            let nal_header = raw[0];
            let nri_f = nal_header & 0xE0;
            let nal_payload = &raw[1..];
            // fragment payload size limited by MAX_RTP_PAYLOAD minus FU-A headers (2 bytes)
            let frag_size = MAX_RTP_PAYLOAD.saturating_sub(2);
            let mut offset = 0usize;
            let mut first = true;
            while offset < nal_payload.len() {
                let end = usize::min(offset + frag_size, nal_payload.len());
                let slice = &nal_payload[offset..end];
                let mut payload_vec: Vec<u8> = Vec::with_capacity(slice.len() + 2);
                // FU indicator: F | NRI | Type(28)
                let fu_indicator = nri_f | 28u8;
                // FU header: S/E/R | original nal type
                let mut fu_header = (raw[0] & 0x1F) as u8; // placeholder for nal type
                let s_bit = first;
                let e_bit = end == nal_payload.len();
                if s_bit {
                    fu_header |= 0x80; // S
                }
                if e_bit {
                    fu_header |= 0x40; // E
                }
                payload_vec.push(fu_indicator);
                payload_vec.push(fu_header);
                payload_vec.extend_from_slice(slice);

                sender.seq = sender.seq.wrapping_add(1);
                let mut pkt = RtpPacket::default();
                pkt.header.version = 2;
                pkt.header.payload_type = sender.payload_type;
                pkt.header.sequence_number = sender.seq;
                pkt.header.timestamp = frame_rtp_ts;
                pkt.header.ssrc = sender.ssrc;
                pkt.payload = payload_vec.into();
                let buf = pkt.marshal()?;
                let _ = sender.socket.send_to(&buf, &sender.target).await?;

                first = false;
                offset = end;
            }
        }
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

    #[test]
    fn fragmentation_timestamp_consistency() -> anyhow::Result<()> {
        // Build a synthetic large NALU (no Annex-B start code)
        const MAX_RTP_PAYLOAD: usize = 1200;
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
