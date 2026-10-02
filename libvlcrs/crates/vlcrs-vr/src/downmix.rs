//! Audio helper: channel downmix to stereo and volume ramping.
//!
//! The Android platform decoders hand back whatever channel count the stream
//! carries (mono, stereo, 5.1, 7.1 …) while AAudio streams are opened with the
//! layout we ask for.  Keeping a simple, well tested downmixer here avoids
//! depending on a resampling library.

/// Standard WAVE channel order used by Android decoders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelLayout {
    /// 1 channel.
    Mono,
    /// L R.
    Stereo,
    /// L R C.
    Lcr,
    /// L R Ls Rs (quad).
    Quad,
    /// L R C LFE Ls Rs (5.1).
    Surround51,
    /// L R C LFE Ls Rs Lr Rr (7.1).
    Surround71,
    /// Anything else: treated as "interleaved, mixed with equal weights".
    Unknown(usize),
}

impl ChannelLayout {
    /// Build from a channel count.
    pub fn from_channels(channels: usize) -> ChannelLayout {
        match channels {
            0 | 1 => ChannelLayout::Mono,
            2 => ChannelLayout::Stereo,
            3 => ChannelLayout::Lcr,
            4 => ChannelLayout::Quad,
            6 => ChannelLayout::Surround51,
            8 => ChannelLayout::Surround71,
            n => ChannelLayout::Unknown(n),
        }
    }

    /// Number of channels.
    pub fn channels(&self) -> usize {
        match self {
            ChannelLayout::Mono => 1,
            ChannelLayout::Stereo => 2,
            ChannelLayout::Lcr => 3,
            ChannelLayout::Quad => 4,
            ChannelLayout::Surround51 => 6,
            ChannelLayout::Surround71 => 8,
            ChannelLayout::Unknown(n) => *n,
        }
    }
}

/// Downmix weight of channel `c` for the left (`right == false`) or right
/// output of a known layout.
fn weight(layout: ChannelLayout, c: usize, right: bool) -> f32 {
    /// -3 dB, used for the centre and surround channels.
    const ATT: f32 = std::f32::consts::FRAC_1_SQRT_2;
    /// `1.0` for the requested side, `0.0` for the other one.
    #[inline]
    fn side(want_right: bool, right: bool) -> f32 {
        if want_right == right {
            1.0
        } else {
            0.0
        }
    }
    match layout {
        ChannelLayout::Mono => 1.0,
        ChannelLayout::Stereo => side(c == 1, right),
        ChannelLayout::Lcr => match c {
            0 => side(false, right),
            1 => side(true, right),
            2 => ATT,
            _ => 0.0,
        },
        ChannelLayout::Quad => match c {
            0 => side(false, right),
            1 => side(true, right),
            2 => if_side(false, right, ATT),
            3 => if_side(true, right, ATT),
            _ => 0.0,
        },
        ChannelLayout::Surround51 => match c {
            0 => side(false, right),
            1 => side(true, right),
            2 => ATT,
            3 => 0.0, // LFE dropped: platform decoders already fold it in
            4 => if_side(false, right, ATT),
            5 => if_side(true, right, ATT),
            _ => 0.0,
        },
        ChannelLayout::Surround71 => match c {
            0 | 6 => side(false, right),
            1 | 7 => side(true, right),
            2 => ATT,
            3 => 0.0,
            4 => if_side(false, right, ATT),
            5 => if_side(true, right, ATT),
            _ => 0.0,
        },
        ChannelLayout::Unknown(n) => {
            // Equal weight spread over the extra channels, keeping L/R distinct
            // when there are at least two of them.
            match n {
                0 => 0.0,
                1 => 1.0,
                _ => match c {
                    0 => side(false, right),
                    1 => side(true, right),
                    _ => 0.5 / (n as f32 - 1.0),
                },
            }
        }
    }
}

/// `gain` on the requested side, `0.0` on the other one.
#[inline]
fn if_side(want_right: bool, right: bool, gain: f32) -> f32 {
    if want_right == right {
        gain
    } else {
        0.0
    }
}

/// Downmix interleaved `i16` PCM into stereo, appending to `out`.
///
/// `frames` is the number of multi-channel frames available in `input`.
pub fn downmix_i16_to_stereo(
    input: &[i16],
    channels: usize,
    frames: usize,
    out: &mut Vec<i16>,
) -> usize {
    let layout = ChannelLayout::from_channels(channels);
    let ch = layout.channels().max(1);
    let available = frames.min(input.len() / ch);
    if available == 0 {
        return 0;
    }
    if ch == 2 {
        // fast path: already stereo
        let n = available * 2;
        out.extend_from_slice(&input[..n.min(input.len())]);
        return available;
    }
    out.reserve(available * 2);
    for f in 0..available {
        let base = f * ch;
        let mut l = 0.0f32;
        let mut r = 0.0f32;
        for c in 0..ch {
            let idx = base + c;
            if idx >= input.len() {
                break;
            }
            let s = input[idx] as f32;
            let wl = weight(layout, c, false);
            if wl != 0.0 {
                l += s * wl;
            }
            let wr = weight(layout, c, true);
            if wr != 0.0 {
                r += s * wr;
            }
        }
        out.push(saturate(l));
        out.push(saturate(r));
    }
    available
}

/// Mono `i16` → stereo by duplication (used when the stream is opened in
/// stereo but the source is mono).
pub fn mono_i16_to_stereo(input: &[i16], out: &mut Vec<i16>) -> usize {
    out.reserve(input.len() * 2);
    for &s in input {
        out.push(s);
        out.push(s);
    }
    input.len()
}

fn saturate(v: f32) -> i16 {
    if !(v.is_finite()) {
        0
    } else if v > i16::MAX as f32 {
        i16::MAX
    } else if v < i16::MIN as f32 {
        i16::MIN
    } else {
        v as i16
    }
}

/// Apply a linear volume (`0.0 … 1.0`) in place.
pub fn apply_volume(samples: &mut [i16], volume: f32) {
    let v = volume.clamp(0.0, 1.0);
    if (v - 1.0).abs() < 1e-4 {
        return;
    }
    for s in samples.iter_mut() {
        *s = saturate(*s as f32 * v);
    }
}

/// Byte order conversion helper: little endian `i16` PCM as delivered by
/// `AMediaCodec` into a sample slice.
pub fn bytes_to_i16_le(bytes: &[u8]) -> impl Iterator<Item = i16> + '_ {
    bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_is_copied_verbatim() {
        let input: Vec<i16> = vec![100, -100, 200, -200];
        let mut out = Vec::new();
        let n = downmix_i16_to_stereo(&input, 2, 2, &mut out);
        assert_eq!(n, 2);
        assert_eq!(out, input);
    }

    #[test]
    fn mono_is_duplicated() {
        let input: Vec<i16> = vec![7, -9];
        let mut out = Vec::new();
        let n = mono_i16_to_stereo(&input, &mut out);
        assert_eq!(n, 2);
        assert_eq!(out, vec![7, 7, -9, -9]);
    }

    #[test]
    fn five_one_downmix_keeps_side_content() {
        // L R C LFE Ls Rs
        let input: Vec<i16> = vec![1000, 2000, 3000, 4000, 5000, 6000];
        let mut out = Vec::new();
        downmix_i16_to_stereo(&input, 6, 1, &mut out);
        assert_eq!(out.len(), 2);
        let l = out[0] as f32;
        let r = out[1] as f32;
        // left = L + 0.707*C + 0.707*Ls
        const ATT: f32 = std::f32::consts::FRAC_1_SQRT_2;
        let expected = 1000.0 + ATT * 3000.0 + ATT * 5000.0;
        assert!((l - expected).abs() < 4.0, "l = {l}, expected {expected}");
        let expected_r = 2000.0 + ATT * 3000.0 + ATT * 6000.0;
        assert!(
            (r - expected_r).abs() < 4.0,
            "r = {r}, expected {expected_r}"
        );
        // the surrounds must actually contribute
        assert!(l > 1000.0 && r > 2000.0);
    }

    #[test]
    fn downmix_never_clips_out_of_range() {
        let input: Vec<i16> = vec![i16::MAX; 6];
        let mut out = Vec::new();
        downmix_i16_to_stereo(&input, 6, 1, &mut out);
        assert_eq!(out[0], i16::MAX);
        assert_eq!(out[1], i16::MAX);
    }

    #[test]
    fn partial_frame_is_ignored() {
        // 5 samples for a 6 channel layout = 0 complete frames
        let input: Vec<i16> = vec![1, 2, 3, 4, 5];
        let mut out = Vec::new();
        let n = downmix_i16_to_stereo(&input, 6, 1, &mut out);
        assert_eq!(n, 0);
        assert!(out.is_empty());
    }

    #[test]
    fn volume_scales_and_saturates() {
        let mut s = vec![1000i16, -1000, i16::MAX];
        apply_volume(&mut s, 0.5);
        assert_eq!(s[0], 500);
        assert_eq!(s[1], -500);
        let mut s2 = vec![i16::MAX];
        apply_volume(&mut s2, 1.0);
        assert_eq!(s2[0], i16::MAX);
        let mut s3 = vec![1000i16];
        apply_volume(&mut s3, 0.0);
        assert_eq!(s3[0], 0);
    }

    #[test]
    fn layout_from_channels() {
        assert_eq!(ChannelLayout::from_channels(1), ChannelLayout::Mono);
        assert_eq!(ChannelLayout::from_channels(6), ChannelLayout::Surround51);
        assert_eq!(ChannelLayout::from_channels(8), ChannelLayout::Surround71);
        assert_eq!(ChannelLayout::from_channels(5), ChannelLayout::Unknown(5));
        assert_eq!(ChannelLayout::Surround51.channels(), 6);
    }

    #[test]
    fn little_endian_decoding() {
        let bytes = [0x34, 0x12, 0xff, 0xff];
        let v: Vec<i16> = bytes_to_i16_le(&bytes).collect();
        assert_eq!(v, vec![0x1234, -1]);
    }
}
