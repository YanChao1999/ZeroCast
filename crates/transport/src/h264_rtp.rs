//! H.264 RTP packetization via the `rtp` crate (RFC 6184).

use anyhow::{Context, Result};
use bytes::Bytes;
use rtp::codecs::h264::H264Payloader;
use rtp::codecs::h264::H264Packet;
use rtp::packetizer::{Depacketizer, Payloader};

const ANNEX_B_START: [u8; 4] = [0, 0, 0, 1];

/// Concatenate Annex-B NALUs into one byte stream for the payloader.
pub fn nalus_to_annex_b(nalus: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for nal in nalus {
        if nal.len() >= 4 && nal[0..4] == ANNEX_B_START {
            out.extend_from_slice(nal);
        } else if nal.len() >= 3 && nal[0..3] == [0, 0, 1] {
            out.extend_from_slice(nal);
        } else {
            out.extend_from_slice(&ANNEX_B_START);
            out.extend_from_slice(nal);
        }
    }
    out
}

/// Split an access unit into RTP payloads (single NAL, STAP-A, or FU-A per RFC 6184).
pub fn packetize_annex_b(annex_b: &[u8], mtu: usize) -> Result<Vec<Vec<u8>>> {
    let payloader = H264Payloader;
    let payloads = payloader
        .payload(mtu, &Bytes::copy_from_slice(annex_b))
        .context("H264 payloader failed")?;
    Ok(payloads.into_iter().map(|p| p.to_vec()).collect())
}

/// Reassembles RTP payloads into one Annex-B access unit (call `finish` on marker).
pub struct FrameAssembler {
    access_unit: Vec<u8>,
    current_nal: Option<Vec<u8>>,
}

impl FrameAssembler {
    pub fn new() -> Self {
        Self {
            access_unit: Vec::new(),
            current_nal: None,
        }
    }

    pub fn reset(&mut self) {
        self.access_unit.clear();
        self.current_nal = None;
    }

    pub fn push_rtp_payload(&mut self, payload: &[u8]) -> Result<()> {
        if payload.is_empty() {
            return Ok(());
        }
        let mut pkt = H264Packet::default();
        pkt.depacketize(&Bytes::copy_from_slice(payload))
            .context("H264 depacketize failed")?;
        let chunk = pkt.payload;
        if chunk.len() >= 4 && chunk[0..4] == ANNEX_B_START {
            if let Some(prev) = self.current_nal.take() {
                self.access_unit.extend_from_slice(&prev);
            }
            self.current_nal = Some(chunk.to_vec());
        } else if let Some(cur) = &mut self.current_nal {
            cur.extend_from_slice(&chunk);
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Vec<u8> {
        if let Some(cur) = self.current_nal.take() {
            self.access_unit.extend_from_slice(&cur);
        }
        std::mem::take(&mut self.access_unit)
    }
}
