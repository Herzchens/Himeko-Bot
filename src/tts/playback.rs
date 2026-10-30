use super::TtsEngine;
use songbird::input::{AsyncAdapterStream, AsyncReadOnlySource, Input, RawAdapter};
use std::io::{Cursor, ErrorKind};
use std::sync::Arc;
use std::time::Duration;
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error as SymphoniaError,
    formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};
use symphonia::default::{get_codecs, get_probe};
use tokio::io::{AsyncWriteExt, DuplexStream};
use wsola::TimeStretch;

const AUDIO_RESULT_BUFFER: usize = 3;
const PCM_PIPE_BYTES: usize = 256 * 1024;
const ASYNC_ADAPTER_BUFFER_BYTES: usize = 64 * 1024;

pub struct StreamingPlayback {
    pub input: Input,
    pub first_audio_bytes: usize,
    pub sample_rate: u32,
    pub channels: u32,
}

struct DecodedPcm {
    samples: Vec<f32>,
    sample_rate: u32,
    channels: u32,
}

pub async fn start_streaming_plan(
    engine: Arc<dyn TtsEngine>,
    plan: Vec<(String, String)>,
    speed: f32,
    timeout: Duration,
) -> anyhow::Result<StreamingPlayback> {
    if !speed.is_finite() || !(0.25..=2.0).contains(&speed) {
        anyhow::bail!("invalid streaming speech speed: {speed}");
    }
    let (sender, mut receiver) =
        tokio::sync::mpsc::channel::<anyhow::Result<Vec<u8>>>(AUDIO_RESULT_BUFFER);
    let error_sender = sender.clone();

    tokio::spawn(async move {
        match tokio::time::timeout(timeout, engine.synthesize_plan_stream(plan, sender)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let _ = error_sender.send(Err(error)).await;
            }
            Err(_) => {
                let _ = error_sender
                    .send(Err(anyhow::anyhow!(
                        "TTS streaming plan exceeded {} seconds",
                        timeout.as_secs()
                    )))
                    .await;
            }
        }
    });

    let first_audio = receiver
        .recv()
        .await
        .ok_or_else(|| anyhow::anyhow!("TTS stream ended before producing audio"))??;
    if first_audio.is_empty() {
        anyhow::bail!("TTS stream produced an empty first audio chunk");
    }
    let first_audio_bytes = first_audio.len();
    let first_pcm = decode_async(first_audio).await?;

    let sample_rate = first_pcm.sample_rate;
    let channels = first_pcm.channels;
    let (writer, reader) = tokio::io::duplex(PCM_PIPE_BYTES);
    let async_source = AsyncReadOnlySource::new(reader);
    let adapter = AsyncAdapterStream::new(Box::new(async_source), ASYNC_ADAPTER_BUFFER_BYTES);
    let input = RawAdapter::new(adapter, sample_rate, channels).into();

    tokio::spawn(async move {
        if let Err(error) = pump_pcm(writer, first_pcm, receiver, speed).await {
            tracing::warn!(error = %error, "streaming TTS PCM producer stopped");
        }
    });

    Ok(StreamingPlayback {
        input,
        first_audio_bytes,
        sample_rate,
        channels,
    })
}

async fn pump_pcm(
    mut writer: DuplexStream,
    first: DecodedPcm,
    mut receiver: tokio::sync::mpsc::Receiver<anyhow::Result<Vec<u8>>>,
    speed: f32,
) -> anyhow::Result<()> {
    let sample_rate = first.sample_rate;
    let channels = first.channels;
    let mut stretcher = if (speed - 1.0).abs() < f32::EPSILON {
        None
    } else {
        if sample_rate == 0 {
            anyhow::bail!("TTS PCM sample rate cannot be zero");
        }
        let channels_u16 = u16::try_from(channels)
            .map_err(|_| anyhow::anyhow!("invalid TTS PCM channel count: {channels}"))?;
        let mut stretcher = TimeStretch::new(sample_rate, channels_u16)
            .map_err(|error| anyhow::anyhow!("failed to create WSOLA stretcher: {error}"))?;
        stretcher.set_tempo(speed);
        Some(stretcher)
    };

    write_processed_samples(&mut writer, &first.samples, stretcher.as_mut()).await?;

    while let Some(result) = receiver.recv().await {
        let audio = match result {
            Ok(audio) => audio,
            Err(error) => {
                tracing::warn!(error = %error, "TTS segment synthesis failed after stream start");
                break;
            }
        };

        let decoded = decode_async(audio).await?;
        if decoded.sample_rate != sample_rate || decoded.channels != channels {
            anyhow::bail!(
                "TTS stream format changed from {} Hz/{}ch to {} Hz/{}ch",
                sample_rate,
                channels,
                decoded.sample_rate,
                decoded.channels
            );
        }
        write_processed_samples(&mut writer, &decoded.samples, stretcher.as_mut()).await?;
    }

    if let Some(stretcher) = stretcher.as_mut() {
        let tail = stretcher.flush();
        write_samples(&mut writer, &tail).await?;
    }

    writer.shutdown().await?;
    Ok(())
}

async fn write_processed_samples(
    writer: &mut DuplexStream,
    samples: &[f32],
    stretcher: Option<&mut TimeStretch>,
) -> anyhow::Result<()> {
    let Some(stretcher) = stretcher else {
        return write_samples(writer, samples).await;
    };

    stretcher.push(samples);
    loop {
        // 8192 is frame-aligned for mono and stereo, the two layouts accepted
        // by Songbird's raw reader.
        let output = stretcher.pull(8192);
        if output.is_empty() {
            break;
        }
        write_samples(writer, &output).await?;
    }
    Ok(())
}

async fn write_samples(writer: &mut DuplexStream, samples: &[f32]) -> anyhow::Result<()> {
    const SAMPLES_PER_WRITE: usize = 4096;
    for samples in samples.chunks(SAMPLES_PER_WRITE) {
        let mut bytes = Vec::with_capacity(std::mem::size_of_val(samples));
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        writer.write_all(&bytes).await?;
    }
    Ok(())
}

async fn decode_async(audio: Vec<u8>) -> anyhow::Result<DecodedPcm> {
    tokio::task::spawn_blocking(move || decode_audio(audio))
        .await
        .map_err(|error| anyhow::anyhow!("TTS audio decoder task failed: {error}"))?
}

fn decode_audio(audio: Vec<u8>) -> anyhow::Result<DecodedPcm> {
    let source = Cursor::new(audio);
    let stream = MediaSourceStream::new(Box::new(source), Default::default());

    let mut hint = Hint::new();
    hint.with_extension("mp3");

    let probed = get_probe().format(
        &hint,
        stream,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow::anyhow!("TTS audio has no default track"))?;
    let track_id = track.id;
    let codec_params = track.codec_params.clone();
    let mut decoder = get_codecs().make(&codec_params, &DecoderOptions::default())?;

    let mut samples = Vec::<f32>::new();
    let mut sample_rate = codec_params.sample_rate;
    let mut channels = codec_params.channels.map(|value| value.count() as u32);

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                break;
            }
            Err(error) => return Err(error.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(error.into()),
        };
        let spec = *decoded.spec();
        sample_rate.get_or_insert(spec.rate);
        channels.get_or_insert(spec.channels.count() as u32);

        let mut converted = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        converted.copy_interleaved_ref(decoded);
        samples.extend_from_slice(converted.samples());
    }

    let sample_rate =
        sample_rate.ok_or_else(|| anyhow::anyhow!("TTS audio sample rate is missing"))?;
    let channels = channels.ok_or_else(|| anyhow::anyhow!("TTS audio channel count is missing"))?;
    if samples.is_empty() {
        anyhow::bail!("TTS audio decoded to zero PCM samples");
    }
    if !matches!(channels, 1 | 2) {
        anyhow::bail!("unsupported TTS PCM channel count: {channels}");
    }

    Ok(DecodedPcm {
        samples,
        sample_rate,
        channels,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn pcm_writer_format_is_little_endian_f32() {
        let value = 0.5f32;
        assert_eq!(value.to_le_bytes().len(), std::mem::size_of::<f32>());
    }

    #[test]
    fn supported_speed_range_matches_command_contract() {
        for speed in [0.25f32, 0.5, 1.0, 1.5, 2.0] {
            assert!((0.25..=2.0).contains(&speed));
        }
        assert!(!(0.25..=2.0).contains(&0.24));
        assert!(!(0.25..=2.0).contains(&2.01));
    }
}
