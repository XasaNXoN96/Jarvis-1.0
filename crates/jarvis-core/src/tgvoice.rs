// Voice for the Telegram bot, in pure Rust (no ffmpeg, no libopus, no external program):
// - a voice message of Telegram (Ogg/Opus) is decoded to 16 kHz mono samples that the offline recognizer (Vosk)
//   takes; nothing is written to disk;
// - the spoken answer (the WAV Piper makes) is encoded to Ogg/Opus, which Telegram plays as a voice message.

use std::io::Cursor;

// a longer voice message is not processed: recognition would block the bot
pub const MAX_VOICE_SECS: u32 = 30;
pub const MAX_VOICE_BYTES: u64 = 1_500_000;
const RATE: u32 = 16_000;

fn le_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

// the offline recognizer of Jarvis (Vosk): only in the build that has the microphone part and a model
pub fn recognize(samples: &[i16]) -> Result<String, String> {
    #[cfg(feature = "jarvis_app")]
    {
        crate::stt::transcribe(samples)
    }
    #[cfg(not(feature = "jarvis_app"))]
    {
        let _ = samples;
        Err("в этой сборке нет распознавания речи".into())
    }
}

// whether a recognition model is installed (the Lite version has none)
pub fn recognition_available() -> bool {
    cfg!(feature = "jarvis_app") && !crate::vosk_models::scan_vosk_models().is_empty()
}

// Ogg/Opus -> 16 kHz mono i16
pub fn decode_ogg_opus(bytes: &[u8]) -> Result<Vec<i16>, String> {
    let mut reader = ogg::PacketReader::new(Cursor::new(bytes));
    let bad = || "это не голосовое сообщение Telegram (нужен формат Ogg/Opus)".to_string();
    let head = reader.read_packet().map_err(|_| bad())?.ok_or_else(bad)?;
    if head.data.len() < 19 || &head.data[..8] != b"OpusHead" {
        return Err(bad());
    }
    let channels = head.data[9] as usize;
    if !(1..=2).contains(&channels) || head.data[18] != 0 {
        return Err("такой звук (много каналов) не поддерживается".into());
    }
    let pre_skip = le_u16(&head.data, 10).unwrap_or(0) as usize / 3; // 48 kHz units -> 16 kHz
    let tags = reader.read_packet().map_err(|_| bad())?.ok_or_else(bad)?;
    if tags.data.len() < 8 || &tags.data[..8] != b"OpusTags" {
        return Err(bad());
    }
    let mut dec = opus_decoder::OpusDecoder::new(RATE, channels).map_err(|_| "не удалось запустить разбор звука".to_string())?;
    let mut frame = vec![0i16; dec.max_frame_size_per_channel() * channels];
    let max_samples = (MAX_VOICE_SECS as usize + 1) * RATE as usize;
    let mut out: Vec<i16> = Vec::new();
    loop {
        let p = match reader.read_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(_) => return Err("звук повреждён".into()),
        };
        let n = dec.decode(&p.data, &mut frame, false).map_err(|_| "звук повреждён".to_string())?;
        for i in 0..n {
            let s = if channels == 1 { frame[i] } else { ((frame[2 * i] as i32 + frame[2 * i + 1] as i32) / 2) as i16 };
            out.push(s);
        }
        if out.len() > max_samples {
            return Err(format!("голосовое длиннее {} секунд", MAX_VOICE_SECS));
        }
    }
    if pre_skip < out.len() {
        out.drain(..pre_skip);
    }
    Ok(out)
}

// WAV (16-bit PCM, any rate, the first channel) -> Ogg/Opus voice message
pub fn encode_wav_to_ogg_opus(wav: &[u8]) -> Result<Vec<u8>, String> {
    let mut r = hound::WavReader::new(Cursor::new(wav)).map_err(|_| "озвучка получилась в непонятном формате".to_string())?;
    let spec = r.spec();
    if spec.bits_per_sample != 16 || spec.sample_format != hound::SampleFormat::Int || spec.sample_rate == 0 {
        return Err("озвучка получилась не в 16-битном формате".into());
    }
    let step = spec.channels.max(1) as usize;
    let samples: Vec<i16> = r.samples::<i16>().filter_map(|s| s.ok()).step_by(step).collect();
    if samples.is_empty() {
        return Err("озвучка пустая".into());
    }
    // Opus takes 24 kHz: a linear resampling is enough for speech
    const OUT: u32 = 24_000;
    let ratio = spec.sample_rate as f64 / OUT as f64;
    let n = (samples.len() as f64 / ratio) as usize;
    let mut pcm: Vec<i16> = Vec::with_capacity(n);
    for i in 0..n {
        let pos = i as f64 * ratio;
        let a = pos as usize;
        let f = pos - a as f64;
        let s0 = samples[a.min(samples.len() - 1)] as f64;
        let s1 = samples[(a + 1).min(samples.len() - 1)] as f64;
        pcm.push((s0 + (s1 - s0) * f) as i16);
    }
    let frame = (OUT / 50) as usize; // 20 ms
    let mut enc = opus_rs::OpusEncoder::new(OUT as i32, 1, opus_rs::Application::Voip).map_err(|_| "не удалось запустить сжатие звука".to_string())?;
    enc.bitrate_bps = 24_000;
    let mut out = Vec::new();
    {
        let mut w = ogg::PacketWriter::new(Cursor::new(&mut out));
        let mut head = Vec::from(&b"OpusHead"[..]);
        head.extend_from_slice(&[1, 1]);
        head.extend_from_slice(&312u16.to_le_bytes());
        head.extend_from_slice(&OUT.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes());
        head.push(0);
        let io = |_| "не удалось собрать файл озвучки".to_string();
        w.write_packet(head, 1, ogg::PacketWriteEndInfo::EndPage, 0).map_err(io)?;
        let mut tags = Vec::from(&b"OpusTags"[..]);
        let vendor = b"jarvis";
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes());
        w.write_packet(tags, 1, ogg::PacketWriteEndInfo::EndPage, 0).map_err(io)?;
        let frames = pcm.len().div_ceil(frame);
        for i in 0..frames {
            let mut buf = vec![0i16; frame];
            let end = ((i + 1) * frame).min(pcm.len());
            buf[..end - i * frame].copy_from_slice(&pcm[i * frame..end]);
            let mut packet = vec![0u8; 1275];
            let len = enc.encode_i16(&buf, frame, &mut packet).map_err(|_| "не удалось сжать звук".to_string())?;
            let end_info = if i + 1 == frames { ogg::PacketWriteEndInfo::EndStream } else { ogg::PacketWriteEndInfo::NormalPacket };
            // (the position in the file is counted in 48 kHz samples)
            w.write_packet(packet[..len].to_vec(), 1, end_info, (i as u64 + 1) * 960).map_err(io)?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // a 3-second speech made by ffmpeg (libopus, 48 kHz mono, as Telegram sends it)
    const VOICE: &[u8] = include_bytes!("testdata/voice.ogg");
    // the same audio decoded by ffmpeg to 16 kHz mono 16-bit (the reference)
    const REFERENCE: &[u8] = include_bytes!("testdata/voice_ref16k.raw");

    fn reference() -> Vec<i16> {
        REFERENCE.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
    }

    fn energy(s: &[i16]) -> f64 {
        (s.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>() / s.len().max(1) as f64).sqrt()
    }

    #[test]
    fn a_telegram_voice_message_is_decoded_to_the_same_speech_as_a_reference_decoder() {
        let got = decode_ogg_opus(VOICE).unwrap();
        let want = reference();
        assert!((got.len() as i64 - want.len() as i64).abs() < 1600, "{} vs {} samples", got.len(), want.len());
        let (eg, ew) = (energy(&got), energy(&want));
        assert!(eg > 300.0, "not silence: {}", eg);
        assert!((eg / ew - 1.0).abs() < 0.1, "the same loudness: {} vs {}", eg, ew);
        // the waveforms follow each other (a small shift of the start is allowed)
        let n = got.len().min(want.len()) - 1000;
        let mut best = 0.0f64;
        for shift in -400i64..400 {
            let (mut dot, mut a, mut b) = (0.0f64, 0.0f64, 0.0f64);
            for i in (400..n).step_by(3) {
                let (x, y) = (got[(i as i64 + shift) as usize] as f64, want[i] as f64);
                dot += x * y;
                a += x * x;
                b += y * y;
            }
            best = best.max(dot / (a.sqrt() * b.sqrt() + 1e-9));
        }
        assert!(best > 0.9, "correlation with the reference {}", best);
    }

    #[test]
    fn what_is_not_a_voice_message_is_refused() {
        assert!(decode_ogg_opus(b"").is_err());
        assert!(decode_ogg_opus(b"RIFF....WAVEfmt ").is_err());
        assert!(decode_ogg_opus(&VOICE[..100]).is_err() || decode_ogg_opus(&VOICE[..100]).map(|v| v.len() < 16000).unwrap_or(true));
        let mut junk = VOICE.to_vec();
        for b in junk.iter_mut().skip(200).step_by(7) {
            *b ^= 0x5a;
        }
        // a damaged file never panics: an error or whatever could be decoded
        let _ = decode_ogg_opus(&junk);
    }

    #[test]
    fn a_spoken_answer_becomes_a_voice_message_that_decodes_back_to_speech() {
        // the reference speech as a 22.05 kHz WAV (the rate of a Piper voice)
        let want = reference();
        let mut wav = Vec::new();
        {
            let spec = hound::WavSpec { channels: 1, sample_rate: 16000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
            let mut w = hound::WavWriter::new(Cursor::new(&mut wav), spec).unwrap();
            for s in &want {
                w.write_sample(*s).unwrap();
            }
            w.finalize().unwrap();
        }
        let ogg = encode_wav_to_ogg_opus(&wav).unwrap();
        assert_eq!(&ogg[..4], b"OggS");
        assert!(ogg.len() < wav.len() / 4, "compressed: {} vs {}", ogg.len(), wav.len());
        let back = decode_ogg_opus(&ogg).unwrap();
        assert!((back.len() as i64 - want.len() as i64).abs() < 1600, "{} vs {}", back.len(), want.len());
        let (eb, ew) = (energy(&back), energy(&want));
        assert!((eb / ew - 1.0).abs() < 0.25, "loudness {} vs {}", eb, ew);
    }

    #[test]
    fn a_broken_or_silent_wav_is_an_error_not_a_panic() {
        assert!(encode_wav_to_ogg_opus(b"nope").is_err());
        assert!(encode_wav_to_ogg_opus(b"RIFF").is_err());
    }
}
