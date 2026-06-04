//! FFmpeg encode → decode integration tests (run on CI with ffmpeg installed).
//!
//! Locally: skipped when `ffmpeg` is not on PATH (unless `CI` is set).

use zerocast_transport::{
    decode_access_unit_rgb24, decoded_cycle_from_rgb, encode_access_unit_oneshot,
    nalus_to_annex_b, test_cycle_frame, FfmpegCliEncoder, VideoEncoder, CYCLE_COUNT,
};

fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn should_run() -> bool {
    std::env::var("CI").is_ok() || ffmpeg_available()
}

/// Non-flat pattern so IDR size is realistic (flat color compresses to a few KB).
fn test_pattern(width: u32, height: u32) -> Vec<u8> {
    let len = (width as usize) * (height as usize) * 3;
    (0..len)
        .map(|i| ((i.wrapping_mul(17)) % 251) as u8)
        .collect()
}

fn nalu_type(nal: &[u8]) -> Option<u8> {
    let hdr = if nal.len() >= 4 && nal[0..4] == [0, 0, 0, 1] {
        4
    } else if nal.len() >= 3 && nal[0..3] == [0, 0, 1] {
        3
    } else {
        return None;
    };
    nal.get(hdr).map(|b| b & 0x1f)
}

fn max_idr_nal_len(nalus: &[Vec<u8>]) -> usize {
    nalus
        .iter()
        .filter(|n| nalu_type(n.as_slice()) == Some(5))
        .map(|n| n.len())
        .max()
        .unwrap_or(0)
}

fn decode_roundtrip(nalus: &[Vec<u8>], width: u32, height: u32) {
    let frame_len = (width as usize) * (height as usize) * 3;
    let annex_b = nalus_to_annex_b(nalus);
    let (rgb, w, h) =
        decode_access_unit_rgb24(&annex_b, width, height).expect("decode access unit");
    assert_eq!((w, h), (width, height));
    assert_eq!(rgb.len(), frame_len);
}

#[test]
fn low_426x240_oneshot_roundtrip() {
    if !should_run() {
        eprintln!("SKIP low_426x240_oneshot_roundtrip: ffmpeg not on PATH");
        return;
    }
    let width = 426u32;
    let height = 240u32;
    let frame = test_pattern(width, height);
    let nalus = encode_access_unit_oneshot(&frame, width, height, 15).expect("oneshot encode");
    assert!(
        nalus.iter().any(|n| matches!(nalu_type(n), Some(7 | 8))),
        "expected SPS/PPS"
    );
    assert!(
        max_idr_nal_len(&nalus) >= 8_000,
        "IDR too small at 426p (got {} B)",
        max_idr_nal_len(&nalus)
    );
    decode_roundtrip(&nalus, width, height);
}

#[test]
fn med_1280x720_oneshot_roundtrip_full_idr() {
    if !should_run() {
        eprintln!("SKIP med_1280x720_oneshot_roundtrip_full_idr: ffmpeg not on PATH");
        return;
    }
    let width = 1280u32;
    let height = 720u32;
    let frame = test_pattern(width, height);
    let mut enc =
        FfmpegCliEncoder::open_with_warmup(width, height, 30, Some(&frame)).expect("open encoder");
    let (nalus, _) = enc.encode_frame(&frame, 0).expect("encode frame");
    let idr_len = max_idr_nal_len(&nalus);
    // Full 720p IDR is typically 50–120 KiB; truncated pipe split was ~32 KiB.
    assert!(
        idr_len >= 40_000,
        "IDR NAL too small ({idr_len} B): regression for 32764 B pipe split / half-green at 720p"
    );
    decode_roundtrip(&nalus, width, height);
}

/// High-level pipeline: 10 distinct frames through live pipe encode → decode (426p).
#[test]
fn ten_frame_cycle_live_pipe_426p() {
    if !should_run() {
        eprintln!("SKIP ten_frame_cycle_live_pipe_426p: ffmpeg not on PATH");
        return;
    }
    const WIDTH: u32 = 426;
    const HEIGHT: u32 = 240;
    const FPS: u32 = 15;

    let frame0 = test_cycle_frame(WIDTH, HEIGHT, 0);
    let mut enc = FfmpegCliEncoder::open_with_warmup(WIDTH, HEIGHT, FPS, Some(&frame0))
        .expect("open live encoder");

    let mut seen_tags = [false; CYCLE_COUNT as usize];

    for frame_index in 0..=CYCLE_COUNT {
        let cycle = frame_index % CYCLE_COUNT;
        let frame = test_cycle_frame(WIDTH, HEIGHT, cycle);
        let (nalus, _) = enc
            .encode_frame(&frame, frame_index as u64)
            .expect("encode frame");
        assert!(
            nalus.iter().any(|n| matches!(nalu_type(n), Some(7 | 8 | 1 | 5))),
            "frame {frame_index}: expected param or VCL NALs"
        );

        let annex_b = nalus_to_annex_b(&nalus);
        let (rgb, w, h) =
            decode_access_unit_rgb24(&annex_b, WIDTH, HEIGHT).expect("decode access unit");
        assert_eq!((w, h), (WIDTH, HEIGHT));

        let decoded_cycle = decoded_cycle_from_rgb(&rgb, WIDTH, HEIGHT);
        assert_eq!(
            decoded_cycle, cycle,
            "frame {frame_index}: tag mismatch (got cycle {decoded_cycle}, expected {cycle})"
        );
        seen_tags[cycle as usize] = true;
    }

    assert!(
        seen_tags.iter().all(|&s| s),
        "expected all 10 cycle tags to be recognized"
    );
}
