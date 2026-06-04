//! Cross-platform H.264 encoding via the `ffmpeg` CLI (Approach A).
//!
//! Uses a long-lived `ffmpeg` pipe when available; falls back to one-shot per frame
//! if the pipe stalls. Requires `ffmpeg` on `PATH` (Windows, macOS, Linux).
//! When `ffmpeg` is missing, a lightweight synthetic encoder is used for tests.

use anyhow::{bail, Context, Result};
use std::io::{Read, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub const DEFAULT_WIDTH: u32 = 640;
pub const DEFAULT_HEIGHT: u32 = 360;
pub const DEFAULT_FPS: u32 = 30;

const ANNEX_B_START_CODE: &[u8] = &[0, 0, 0, 1];

const PIPE_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(800);
/// Collect trailing slice NAL batches for the same picture (pipe emits per VCL NAL).
const PIPE_SLICE_DRAIN: Duration = Duration::from_millis(50);
const WARMUP_FRAME_COUNT: usize = 2;
/// Trait boundary for swapping CLI encoding with platform HW encoders later.
pub trait VideoEncoder {
    fn encode_frame(&mut self, rgb24: &[u8], frame_index: u64) -> Result<(Vec<Vec<u8>>, u128)>;
}

/// One-shot encode (opens a persistent encoder, encodes one frame, then drops).
#[allow(dead_code)]
pub fn encode_frame_to_h264_annexb(
    rgb24: &[u8],
    width: u32,
    height: u32,
    frame_index: u64,
) -> Result<(Vec<Vec<u8>>, u128)> {
    let mut enc = FfmpegCliEncoder::open(width, height, DEFAULT_FPS)?;
    enc.encode_frame(rgb24, frame_index)
}

enum EncoderBackend {
    Pipe {
        child: Child,
        stdin: ChildStdin,
        frame_rx: Receiver<Vec<Vec<u8>>>,
        reader_handle: JoinHandle<()>,
    },
    /// One `ffmpeg` process per frame (fallback when the live pipe stalls).
    Oneshot,
    Synthetic,
}

/// Long-lived `ffmpeg` subprocess tuned for low-latency LAN streaming.
pub struct FfmpegCliEncoder {
    width: u32,
    height: u32,
    fps: u32,
    backend: EncoderBackend,
    /// Cached SPS/PPS (Annex-B) for one-shot mode — prepended to each VCL access unit.
    param_nals: Vec<Vec<u8>>,
}

impl FfmpegCliEncoder {
    /// Test/dev fallback when `ffmpeg` is not installed.
    pub fn open_synthetic(width: u32, height: u32, fps: u32) -> Self {
        Self {
            width,
            height,
            fps,
            backend: EncoderBackend::Synthetic,
            param_nals: Vec::new(),
        }
    }

    pub fn open(width: u32, height: u32, fps: u32) -> Result<Self> {
        Self::open_with_warmup(width, height, fps, None)
    }

    /// Open encoder and warm the pipe with `warmup_rgb24` (or black if `None`).
    pub fn open_with_warmup(
        width: u32,
        height: u32,
        fps: u32,
        warmup_rgb24: Option<&[u8]>,
    ) -> Result<Self> {
        #[cfg(feature = "libav")]
        {
            let _ = (width, height, fps, warmup_rgb24);
            bail!("disable the `libav` feature to use the ffmpeg CLI encoder");
        }

        let mut param_nals = Vec::new();
        if let Some(warmup) = warmup_rgb24 {
            match prime_param_nals(warmup, width, height, fps) {
                Ok(nals) if !nals.is_empty() => param_nals = nals,
                Ok(_) => eprintln!("encoder: warning: could not cache SPS/PPS from warmup"),
                Err(e) => eprintln!("encoder: warning: SPS/PPS prime failed: {e:#}"),
            }
        }

        let use_pipe = use_live_pipe_encoder(width, height);
        if !use_pipe {
            eprintln!(
                "encoder: one-shot ffmpeg ({}x{} @ {} fps; pipe parser unsafe above 640x360)",
                width, height, fps
            );
            return Ok(Self {
                width,
                height,
                fps,
                backend: EncoderBackend::Oneshot,
                param_nals,
            });
        }

        match try_spawn_ffmpeg(width, height, fps, warmup_rgb24) {
            Ok(backend) => {
                eprintln!("encoder: live ffmpeg pipe ({}x{} @ {} fps)", width, height, fps);
                Ok(Self {
                    width,
                    height,
                    fps,
                    backend,
                    param_nals,
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                width,
                height,
                fps,
                backend: EncoderBackend::Synthetic,
                param_nals: Vec::new(),
            }),
            Err(e) => {
                eprintln!("encoder: pipe spawn failed ({e}), using one-shot ffmpeg");
                Ok(Self {
                    width,
                    height,
                    fps,
                    backend: EncoderBackend::Oneshot,
                    param_nals,
                })
            }
        }
    }

    fn switch_to_oneshot(&mut self) {
        if let EncoderBackend::Pipe {
            mut child,
            reader_handle,
            ..
        } = std::mem::replace(&mut self.backend, EncoderBackend::Oneshot)
        {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader_handle.join();
        }
        eprintln!("encoder: switched to one-shot ffmpeg");
    }

    fn frame_bytes(&self) -> usize {
        (self.width as usize) * (self.height as usize) * 3
    }

    fn pts_ns(&self, frame_index: u64) -> u128 {
        (frame_index as u128)
            .saturating_mul(1_000_000_000u128)
            / (self.fps.max(1) as u128)
    }

    /// Prepend cached SPS/PPS when the access unit has VCL but no parameter sets.
    ///
    /// The recv path uses a stateless one-shot ffmpeg decoder per frame, so every
    /// RTP access unit that contains slice/IDR data must include SPS/PPS (not only
    /// on GOP boundaries).
    fn maybe_attach_params(&self, nalus: Vec<Vec<u8>>) -> Result<Vec<Vec<u8>>> {
        attach_param_nals(&self.param_nals, nalus)
    }
}

impl VideoEncoder for FfmpegCliEncoder {
    fn encode_frame(&mut self, rgb24: &[u8], frame_index: u64) -> Result<(Vec<Vec<u8>>, u128)> {
        let expected = self.frame_bytes();
        if rgb24.len() != expected {
            bail!(
                "frame size mismatch: got {} bytes, expected {} ({}x{} RGB24)",
                rgb24.len(),
                expected,
                self.width,
                self.height
            );
        }

        match &mut self.backend {
            EncoderBackend::Synthetic => Ok((
                synthetic_nalus(self.width, self.height, frame_index),
                self.pts_ns(frame_index),
            )),
            EncoderBackend::Oneshot => {
                let nalus = encode_frame_oneshot(rgb24, self.width, self.height, self.fps)?;
                let inline_params: Vec<Vec<u8>> = nalus
                    .iter()
                    .filter(|n| matches!(nalu_type_of(n), Some(7 | 8)))
                    .cloned()
                    .collect();
                if !inline_params.is_empty() {
                    self.param_nals = inline_params;
                } else if self.param_nals.is_empty() {
                    if let Ok(nals) = prime_param_nals(rgb24, self.width, self.height, self.fps) {
                        self.param_nals = nals;
                    }
                }
                let nalus = self.maybe_attach_params(nalus)?;
                Ok((nalus, self.pts_ns(frame_index)))
            }
            EncoderBackend::Pipe { stdin, frame_rx, .. } => {
                stdin
                    .write_all(rgb24)
                    .context("failed to write frame to ffmpeg stdin")?;
                stdin.flush().context("failed to flush ffmpeg stdin")?;

                match frame_rx.recv_timeout(PIPE_ATTEMPT_TIMEOUT) {
                    Ok(nalus) if !nalus.is_empty() => {
                        let nalus = drain_pipe_nal_batches(nalus, frame_rx);
                        let nalus = self.maybe_attach_params(nalus)?;
                        Ok((nalus, self.pts_ns(frame_index)))
                    }
                    Ok(_) | Err(RecvTimeoutError::Timeout) => {
                        self.switch_to_oneshot();
                        let nalus = encode_frame_oneshot(rgb24, self.width, self.height, self.fps)?;
                        let nalus = self.maybe_attach_params(nalus)?;
                        Ok((nalus, self.pts_ns(frame_index)))
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        bail!("ffmpeg stdout reader exited unexpectedly");
                    }
                }
            }
        }
    }
}

impl Drop for FfmpegCliEncoder {
    fn drop(&mut self) {
        if let EncoderBackend::Pipe {
            mut child,
            reader_handle,
            ..
        } = std::mem::replace(&mut self.backend, EncoderBackend::Synthetic)
        {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader_handle.join();
        }
    }
}

/// Extract SPS/PPS from a one-frame encode (default x264 emits param sets only).
fn prime_param_nals(rgb24: &[u8], width: u32, height: u32, fps: u32) -> Result<Vec<Vec<u8>>> {
    let nalus = run_oneshot_ffmpeg(rgb24, width, height, fps, OneshotMode::ParamSets)?;
    let params: Vec<Vec<u8>> = nalus
        .into_iter()
        .filter(|n| matches!(nalu_type_of(n), Some(7 | 8)))
        .collect();
    if params.is_empty() {
        bail!("ffmpeg param-set prime produced no SPS/PPS");
    }
    Ok(params)
}

enum OneshotMode {
    /// Default x264 headers — often SPS/PPS only for a single `-frames:v 1`.
    ParamSets,
    /// One access unit with matching SPS/PPS + slice from a single encode.
    WithHeaders,
}

fn oneshot_x264_params(mode: OneshotMode) -> &'static str {
    match mode {
        OneshotMode::ParamSets => {
            "annexb=1:sliced-threads=0:sync-lookahead=0:slices=1:slice-max-size=0"
        }
        OneshotMode::WithHeaders => {
            "repeat-headers=1:annexb=1:sliced-threads=0:sync-lookahead=0:slices=1:slice-max-size=0"
        }
    }
}

fn run_oneshot_ffmpeg(
    rgb24: &[u8],
    width: u32,
    height: u32,
    fps: u32,
    mode: OneshotMode,
) -> Result<Vec<Vec<u8>>> {
    let size = format!("{width}x{height}");
    let fps_s = fps.to_string();
    let gop = fps.max(1).to_string();
    let mut args: Vec<&str> = vec![
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgb24",
        "-s",
        &size,
        "-r",
        &fps_s,
        "-i",
        "-",
        "-frames:v",
        "1",
        "-an",
        "-vf",
        "format=yuv420p",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-preset",
        "ultrafast",
        "-tune",
        "zerolatency",
        "-profile:v",
        "baseline",
        "-bf",
        "0",
        "-g",
        &gop,
        "-keyint_min",
        &gop,
        "-threads",
        "1",
        "-slices",
        "1",
    ];
    let x264_owned = oneshot_x264_params(mode).to_string();
    args.push("-x264-params");
    args.push(&x264_owned);
    args.extend(["-bsf:v", "h264_mp4toannexb", "-f", "h264", "-"]);

    let mut child = Command::new("ffmpeg");
    child
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = child.spawn().context("failed to spawn one-shot ffmpeg")?;
    {
        let mut stdin = child.stdin.take().context("ffmpeg stdin missing")?;
        stdin.write_all(rgb24)?;
    }
    let output = child
        .wait_with_output()
        .context("failed to read one-shot ffmpeg output")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("one-shot ffmpeg failed: {stderr}");
    }
    let nalus = split_annex_b_nalus(&output.stdout);
    if nalus.is_empty() {
        bail!("one-shot ffmpeg produced no NALUs");
    }
    Ok(filter_stream_nalus(nalus))
}

/// One ffmpeg process per frame (slower, but reliable on Windows pipes).
/// One-shot libx264 encode (full access unit). Used by integration tests.
pub fn encode_access_unit_oneshot(
    rgb24: &[u8],
    width: u32,
    height: u32,
    fps: u32,
) -> Result<Vec<Vec<u8>>> {
    encode_frame_oneshot(rgb24, width, height, fps)
}

fn encode_frame_oneshot(
    rgb24: &[u8],
    width: u32,
    height: u32,
    fps: u32,
) -> Result<Vec<Vec<u8>>> {
    let nalus = run_oneshot_ffmpeg(rgb24, width, height, fps, OneshotMode::WithHeaders)?;
    if !nalus.iter().any(|n| nalu_type_of(n).map(is_vcl_nal_type).unwrap_or(false)) {
        bail!(
            "one-shot ffmpeg produced no VCL NAL (types: {:?})",
            nalus
                .iter()
                .filter_map(|n| nalu_type_of(n))
                .collect::<Vec<_>>()
        );
    }
    Ok(nalus)
}

fn attach_param_nals(param_nals: &[Vec<u8>], mut vcl_nalus: Vec<Vec<u8>>) -> Result<Vec<Vec<u8>>> {
    let has_params = vcl_nalus
        .iter()
        .any(|n| matches!(nalu_type_of(n), Some(7 | 8)));
    let has_vcl = vcl_nalus
        .iter()
        .any(|n| nalu_type_of(n).map(is_vcl_nal_type).unwrap_or(false));
    if !has_vcl {
        bail!("encode output has no VCL NAL");
    }
    if has_params || param_nals.is_empty() {
        return Ok(vcl_nalus);
    }
    let mut out = param_nals.to_vec();
    out.append(&mut vcl_nalus);
    Ok(out)
}

fn nalu_type_of(nalu: &[u8]) -> Option<u8> {
    let start = find_annex_b_start(nalu)?;
    let header = start + start_code_len_at(nalu, start);
    nalu.get(header).map(|b| b & 0x1F)
}

fn is_vcl_nal_type(nal_type: u8) -> bool {
    matches!(nal_type, 1 | 5)
}

/// Drop filler/SEI/AUD NALs that confuse the one-shot ffmpeg decoder.
fn filter_stream_nalus(nalus: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    nalus
        .into_iter()
        .filter(|n| !matches!(nalu_type_of(n), Some(6 | 9 | 10 | 11 | 12)))
        .collect()
}

fn try_spawn_ffmpeg(
    width: u32,
    height: u32,
    fps: u32,
    warmup_rgb24: Option<&[u8]>,
) -> std::io::Result<EncoderBackend> {
    let size = format!("{width}x{height}");
    let fps_s = fps.to_string();
    #[cfg(windows)]
    let gop = "1".to_string();
    #[cfg(not(windows))]
    let gop = fps.max(1).to_string();
    #[cfg(windows)]
    let x264_params = "repeat-headers=0:annexb=1:nal-hrd=none:sync-lookahead=0:sliced-threads=0:slices=1:slice-max-size=0";
    #[cfg(not(windows))]
    let x264_params = "repeat-headers=1:annexb=1:nal-hrd=none:sync-lookahead=0:sliced-threads=0:slices=1:slice-max-size=0";

    // `-` is more reliable than `pipe:0` / `pipe:1` on Windows for subprocess pipes.
    let mut child = Command::new("ffmpeg");
    child
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-fflags",
            "nobuffer+flush_packets",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-s",
            &size,
            "-r",
            &fps_s,
            "-i",
            "-",
            "-an",
            "-sn",
            "-dn",
            // rgb24 is 4:4:4; baseline profile requires 4:2:0 (yuv420p).
            "-vf",
            "format=yuv420p",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-preset",
            "ultrafast",
            "-tune",
            "zerolatency",
            "-profile:v",
            "baseline",
            "-bf",
            "0",
            "-g",
            &gop,
            "-keyint_min",
            &gop,
            "-sc_threshold",
            "0",
            "-threads",
            "1",
            "-slices",
            "1",
            "-x264-params",
            x264_params,
            "-flags",
            "+low_delay",
            "-bsf:v",
            "h264_mp4toannexb",
            "-f",
            "h264",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = child.spawn()?;
    let mut stdin = child.stdin.take().expect("ffmpeg stdin");
    let stdout = child.stdout.take().expect("ffmpeg stdout");
    if let Some(stderr) = child.stderr.take() {
        thread::spawn(move || stderr_drain_loop(stderr));
    }
    let (tx, frame_rx) = mpsc::channel();
    let reader_handle = thread::spawn(move || stdout_reader_loop(stdout, tx));

    // Warm up the pipe encoder (>=2 frames); must run right before real capture (see stream_loop).
    let frame_len = (width as usize) * (height as usize) * 3;
    let black = vec![0u8; frame_len];
    let warmup = match warmup_rgb24 {
        Some(f) if f.len() == frame_len => f,
        Some(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "warmup frame size mismatch",
            ));
        }
        None => &black[..],
    };
    for _ in 0..WARMUP_FRAME_COUNT {
        stdin.write_all(warmup)?;
        stdin.flush()?;
    }
    // Let the reader flush warmup NALUs (non-blocking drain).
    thread::sleep(Duration::from_millis(150));
    while frame_rx.try_recv().is_ok() {}

    Ok(EncoderBackend::Pipe {
        child,
        stdin,
        frame_rx,
        reader_handle,
    })
}

/// Merge per-slice batches from the pipe reader into one access unit per captured frame.
fn drain_pipe_nal_batches(mut nalus: Vec<Vec<u8>>, frame_rx: &Receiver<Vec<Vec<u8>>>) -> Vec<Vec<u8>> {
    let deadline = std::time::Instant::now() + PIPE_SLICE_DRAIN;
    while std::time::Instant::now() < deadline {
        match frame_rx.try_recv() {
            Ok(batch) => nalus.extend(batch),
            Err(_) => std::thread::sleep(Duration::from_millis(2)),
        }
    }
    coalesce_adjacent_vcl_nalus(nalus)
}

fn stderr_drain_loop(mut stderr: ChildStderr) {
    let mut buf = [0u8; 4096];
    loop {
        match stderr.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if let Ok(text) = std::str::from_utf8(&buf[..n]) {
                    for line in text.lines() {
                        if !line.is_empty() {
                            eprintln!("ffmpeg: {line}");
                        }
                    }
                }
            }
        }
    }
}

fn stdout_reader_loop(mut stdout: ChildStdout, tx: Sender<Vec<Vec<u8>>>) {
    let mut buf = Vec::with_capacity(512 * 1024);
    let mut scratch = vec![0u8; 256 * 1024];
    let mut pending: Vec<Vec<u8>> = Vec::new();

    loop {
        match stdout.read(&mut scratch) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&scratch[..n]);
                drain_complete_nalus(&mut buf, &mut pending, &tx);
                drain_avcc_nalus(&mut buf, &mut pending, &tx);
            }
            Err(_) => break,
        }
    }

    drain_complete_nalus(&mut buf, &mut pending, &tx);
    drain_avcc_nalus(&mut buf, &mut pending, &tx);
    try_emit_pending(&mut pending, &tx);
}

fn try_emit_pending(pending: &mut Vec<Vec<u8>>, tx: &Sender<Vec<Vec<u8>>>) {
    if pending.is_empty() {
        return;
    }
    let has_vcl = pending
        .iter()
        .any(|n| nalu_type_of(n).map(is_vcl_nal_type).unwrap_or(false));
    if has_vcl {
        let _ = tx.send(std::mem::take(pending));
    }
}

fn push_stream_nalu(pending: &mut Vec<Vec<u8>>, nalu: Vec<u8>, tx: &Sender<Vec<Vec<u8>>>) {
    if matches!(nalu_type_of(&nalu), Some(6 | 9 | 10 | 11 | 12)) {
        return;
    }
    pending.push(nalu);
    try_emit_pending(pending, tx);
}

/// Parse complete NALUs from `buf`; emit a batch when a VCL NAL (slice/IDR) is completed.
fn drain_complete_nalus(buf: &mut Vec<u8>, pending: &mut Vec<Vec<u8>>, tx: &Sender<Vec<Vec<u8>>>) {
    loop {
        let Some((nalu, consumed)) = take_first_nalu(buf) else {
            break;
        };
        buf.drain(..consumed);
        push_stream_nalu(pending, nalu, tx);
    }
}

/// AVCC length-prefixed NALUs (fallback when Annex-B start codes are absent).
fn drain_avcc_nalus(buf: &mut Vec<u8>, pending: &mut Vec<Vec<u8>>, tx: &Sender<Vec<Vec<u8>>>) {
    if find_annex_b_start(buf).is_some() {
        return;
    }
    loop {
        if buf.len() < 4 {
            break;
        }
        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        if len == 0 || len > buf.len().saturating_sub(4) {
            break;
        }
        let mut nalu = vec![0, 0, 0, 1];
        nalu.extend_from_slice(&buf[4..4 + len]);
        buf.drain(..4 + len);
        push_stream_nalu(pending, nalu, tx);
    }
}

/// Rejoin VCL NALs split on a false `0x000001` inside the RBSP (common at 720p+).
fn coalesce_adjacent_vcl_nalus(nalus: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    for nal in nalus {
        if nalu_type_of(&nal).map(is_vcl_nal_type).unwrap_or(false) {
            if let Some(last) = out.last_mut() {
                if nalu_type_of(last).map(is_vcl_nal_type).unwrap_or(false) {
                    last.extend_from_slice(vcl_rbsp_continuation(&nal));
                    continue;
                }
            }
        }
        out.push(nal);
    }
    out
}

fn nal_body(nalu: &[u8]) -> &[u8] {
    let start = find_annex_b_start(nalu).unwrap_or(0);
    let header_end = start + start_code_len_at(nalu, start);
    &nalu[header_end.min(nalu.len())..]
}

/// RBSP bytes after the NAL header byte (for joining a falsely split VCL NAL).
fn vcl_rbsp_continuation(nalu: &[u8]) -> &[u8] {
    let body = nal_body(nalu);
    if body.len() > 1 {
        &body[1..]
    } else {
        body
    }
}

fn nal_type_at(buf: &[u8], start: usize) -> Option<u8> {
    let header_end = start + start_code_len_at(buf, start);
    buf.get(header_end).map(|b| b & 0x1f)
}

fn is_valid_nalu_start_at(buf: &[u8], start: usize) -> bool {
    matches!(nal_type_at(buf, start), Some(1 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12))
}

/// Boundaries inside RBSP: ignore type 1 (`0x000001` + 0x41…) — common false positive in IDR data.
fn is_probable_nalu_boundary(buf: &[u8], start: usize) -> bool {
    is_valid_nalu_start_at(buf, start)
        && matches!(nal_type_at(buf, start), Some(5 | 6 | 7 | 8 | 9))
}

fn use_live_pipe_encoder(width: u32, height: u32) -> bool {
    (width as u64) * (height as u64) <= 640 * 360
}

fn find_next_nalu_start(buf: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 3 < buf.len() {
        let four = i + 4 <= buf.len() && buf[i..i + 4] == *ANNEX_B_START_CODE;
        let three = buf[i..i + 3] == [0, 0, 1] && (i == 0 || buf[i - 1] != 0);
        if (four || three) && is_probable_nalu_boundary(buf, i) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn take_first_nalu(buf: &[u8]) -> Option<(Vec<u8>, usize)> {
    let start = find_annex_b_start(buf)?;
    if !is_valid_nalu_start_at(buf, start) {
        return None;
    }
    let header_end = start + start_code_len_at(buf, start);
    if header_end > buf.len() {
        return None;
    }
    let end = find_next_nalu_start(buf, header_end).unwrap_or(buf.len());
    if end <= start {
        return None;
    }
    Some((buf[start..end].to_vec(), end))
}

fn start_code_len_at(buf: &[u8], start: usize) -> usize {
    if buf.get(start..start + 4) == Some(ANNEX_B_START_CODE) {
        4
    } else {
        3
    }
}

fn find_annex_b_start(buf: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i + 3 <= buf.len() {
        if i + 4 <= buf.len() && buf[i..i + 4] == *ANNEX_B_START_CODE {
            return Some(i);
        }
        if buf[i..i + 3] == [0, 0, 1] && (i == 0 || buf[i - 1] != 0) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn synthetic_nalus(width: u32, height: u32, frame_index: u64) -> Vec<Vec<u8>> {
    let mut nalus = Vec::new();
    nalus.push(vec![0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1f]);
    let mut idr = vec![0, 0, 0, 1, 0x65];
    let payload_len = ((width as usize * height as usize) / 8).max(1400);
    idr.extend(std::iter::repeat((frame_index & 0xff) as u8).take(payload_len));
    nalus.push(idr);
    nalus
}

/// Split a byte stream into Annex-B NALUs (each includes its start code).
pub fn split_annex_b_nalus(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut buf = bytes.to_vec();
    let mut out = Vec::new();
    while let Some((nalu, consumed)) = take_first_nalu(&buf) {
        buf.drain(..consumed);
        out.push(nalu);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annex_b_split_roundtrip() {
        let data = [0u8, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x65, 0x88];
        let nalus = split_annex_b_nalus(&data);
        assert_eq!(nalus.len(), 2);
        assert_eq!(&nalus[0][..5], &[0, 0, 0, 1, 0x67]);
    }

    #[test]
    fn annex_b_split_ignores_false_start_in_rbsp() {
        let mut idr = vec![0u8, 0, 0, 1, 0x65, 0x88];
        idr.extend(std::iter::repeat(0u8).take(32_760));
        // Type-1-like false boundary (common inside large IDR RBSP).
        idr.extend_from_slice(&[0, 0, 0, 1, 0x41, 0xaa]);
        let nalus = split_annex_b_nalus(&idr);
        assert_eq!(nalus.len(), 1, "false type-1 start inside IDR must not split NAL");
    }

    #[test]
    fn coalesce_adjacent_idr_parts() {
        let a = vec![0u8, 0, 0, 1, 0x65, 1, 2, 3];
        let b = vec![0u8, 0, 0, 1, 0x65, 4, 5, 6];
        let merged = coalesce_adjacent_vcl_nalus(vec![a, b]);
        assert_eq!(merged.len(), 1);
        assert_eq!(&merged[0][5..], &[1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn annex_b_split_mixed_start_code_lengths() {
        let mut data = vec![0u8, 0, 0, 1, 0x67, 0x42];
        data.extend_from_slice(&[0, 0, 1, 0x68, 0xee, 0xde]);
        data.extend_from_slice(&[0, 0, 1, 0x65, 0x88, 0x99]);
        let nalus = split_annex_b_nalus(&data);
        assert_eq!(nalus.len(), 3);
        assert_eq!(nalu_type_of(&nalus[0]), Some(7));
        assert_eq!(nalu_type_of(&nalus[1]), Some(8));
        assert_eq!(nalu_type_of(&nalus[2]), Some(5));
        assert!(nalus[1].len() < 32, "PPS should be tiny, not merged with IDR");
    }

    #[test]
    #[ignore = "requires ffmpeg on PATH"]
    fn ffmpeg_live_encode() {
        let mut enc = FfmpegCliEncoder::open(64, 48, 30).expect("open ffmpeg");
        let frame = vec![0u8; 64 * 48 * 3];
        let (nalus, _) = enc.encode_frame(&frame, 0).expect("encode frame");
        assert!(!nalus.is_empty());
        assert!(
            nalus
                .iter()
                .any(|n| nalu_type_of(n).map(is_vcl_nal_type).unwrap_or(false)),
            "expected a VCL NAL in output"
        );
    }

    #[test]
    #[ignore = "requires ffmpeg on PATH"]
    fn prime_param_nals_are_small_at_1080p() {
        let frame = vec![0x80u8; 1920 * 1080 * 3];
        let params = prime_param_nals(&frame, 1920, 1080, 60).expect("prime");
        for n in &params {
            let t = nalu_type_of(n).expect("nal type");
            assert!(
                n.len() < 4096,
                "param nal type {t} too large: {} bytes",
                n.len()
            );
        }
    }

    #[test]
    #[ignore = "requires ffmpeg on PATH"]
    fn oneshot_encode_decode_roundtrip_426x240() {
        use crate::decode_access_unit_rgb24;
        use crate::h264_rtp::nalus_to_annex_b;

        let frame = vec![0x80u8; 426 * 240 * 3];
        let vcl = encode_frame_oneshot(&frame, 426, 240, 15).expect("oneshot encode");
        assert!(
            vcl.iter()
                .any(|n| matches!(nalu_type_of(n), Some(7 | 8))),
            "expected inline SPS/PPS"
        );
        let vcl_count = vcl
            .iter()
            .filter(|n| nalu_type_of(n).map(is_vcl_nal_type).unwrap_or(false))
            .count();
        assert_eq!(vcl_count, 1, "expected one VCL NAL per frame");
        let annex_b = nalus_to_annex_b(&vcl);
        decode_access_unit_rgb24(&annex_b, 426, 240).expect("decode roundtrip");
    }

    #[test]
    #[ignore = "requires ffmpeg on PATH"]
    fn oneshot_vcl_frame_includes_slice() {
        let frame = vec![0x80u8; 426 * 240 * 3];
        let nalus = encode_frame_oneshot(&frame, 426, 240, 15).expect("oneshot encode");
        assert!(nalus.iter().any(|n| nalu_type_of(n) == Some(7)));
        assert!(nalus.iter().any(|n| nalu_type_of(n) == Some(8)));
        assert_eq!(
            nalus
                .iter()
                .filter(|n| nalu_type_of(n).map(is_vcl_nal_type).unwrap_or(false))
                .count(),
            1
        );
    }

    #[test]
    #[ignore = "requires ffmpeg on PATH"]
    fn ffmpeg_live_encode_640x360() {
        let frame = vec![0u8; 640 * 360 * 3];
        let mut enc =
            FfmpegCliEncoder::open_with_warmup(640, 360, 30, Some(&frame)).expect("open ffmpeg");
        let (nalus, _) = enc.encode_frame(&frame, 0).expect("encode frame");
        assert!(!nalus.is_empty());
    }

    #[test]
    fn synthetic_encoder_produces_vcl() {
        let mut enc = FfmpegCliEncoder::open_synthetic(64, 48, 30);
        let frame = vec![0u8; 64 * 48 * 3];
        let (nalus, pts) = enc.encode_frame(&frame, 0).unwrap();
        assert!(!nalus.is_empty());
        assert_eq!(pts, 0);
    }
}
