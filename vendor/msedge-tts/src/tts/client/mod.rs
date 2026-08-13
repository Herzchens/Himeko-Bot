//! TTS Client module

use crate::tts::AudioMetadata;

/// Synthesized Audio and Metadata
#[derive(Debug)]
pub struct SynthesizedAudio {
    pub audio_format: String,
    pub audio_bytes: Vec<u8>,
    pub audio_metadata: Vec<AudioMetadata>,
}

/// Async Client
#[cfg(any(feature = "smol-runtime", feature = "tokio-runtime"))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(feature = "smol-runtime", feature = "tokio-runtime")))
)]
pub struct MSEdgeTTSClientAsync<T>(pub(crate) async_tungstenite::WebSocketStream<T>);

#[cfg(any(feature = "smol-runtime", feature = "tokio-runtime"))]
impl<T: futures_util::io::AsyncRead + futures_util::io::AsyncWrite + Unpin>
    MSEdgeTTSClientAsync<T>
{
    /// Synthesize text to speech with a [crate::tts::SpeechConfig] asynchronously
    pub async fn synthesize(
        &mut self,
        text: &str,
        config: &crate::tts::SpeechConfig,
    ) -> crate::error::Result<SynthesizedAudio> {
        use futures_util::StreamExt;
        let config_message = crate::tts::build_config_message(config);
        let ssml_message = crate::tts::build_ssml_message(text, config);
        self.0.send(config_message).await?;
        self.0.send(ssml_message).await?;

        let mut audio_bytes = Vec::new();
        let mut audio_metadata = Vec::new();
        let mut turn_start = false;
        let mut response = false;
        let mut turn_end = false;
        loop {
            if turn_end {
                break;
            }

            if let Some(message) = self.0.next().await {
                let message = message?;
                let payload = crate::tts::Payload::process(
                    message,
                    &mut turn_start,
                    &mut response,
                    &mut turn_end,
                )?;
                if let Some(payload) = payload {
                    match payload {
                        crate::tts::Payload::AudioBytes(payload) => {
                            audio_bytes.push(payload);
                        }
                        crate::tts::Payload::AudioMetadata(metadata) => {
                            audio_metadata.extend(metadata);
                        }
                    }
                }
            }
        }

        let audio_bytes = audio_bytes
            .iter()
            .flat_map(|(bytes, index)| &bytes[*index..])
            .copied()
            .collect();

        Ok(SynthesizedAudio {
            audio_format: config.audio_format.clone(),
            audio_bytes,
            audio_metadata,
        })
    }
}

#[cfg(feature = "blocking")]
mod blocking;
#[cfg(feature = "blocking")]
#[cfg_attr(docsrs, doc(cfg(feature = "blocking")))]
pub use blocking::*;

#[cfg(feature = "smol-runtime")]
#[cfg_attr(docsrs, doc(cfg(feature = "smol-runtime")))]
pub mod smol_runtime;

#[cfg(feature = "tokio-runtime")]
#[cfg_attr(docsrs, doc(cfg(feature = "tokio-runtime")))]
pub mod tokio_runtime;

#[cfg(all(test, feature = "tokio-runtime"))]
mod eof_fault_tests {
    use super::*;
    use futures_util::io::{AsyncRead, AsyncWrite};
    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    #[derive(Default)]
    struct ImmediateEofTransport;

    impl AsyncRead for ImmediateEofTransport {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Ready(Ok(0))
        }
    }

    impl AsyncWrite for ImmediateEofTransport {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn config() -> crate::tts::SpeechConfig {
        crate::tts::SpeechConfig {
            voice_name: "vi-VN-HoaiMyNeural".to_string(),
            audio_format: "audio-24khz-48kbitrate-mono-mp3".to_string(),
            pitch: 0,
            rate: 0,
            volume: 0,
        }
    }

    #[test]
    fn abrupt_transport_eof_returns_error_without_liveness_spin() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime must build");
        runtime.block_on(async {
            let websocket = async_tungstenite::WebSocketStream::from_raw_socket(
                ImmediateEofTransport,
                tungstenite::protocol::Role::Client,
                None,
            )
            .await;
            let mut client = MSEdgeTTSClientAsync(websocket);
            let result = tokio::time::timeout(
                Duration::from_millis(250),
                client.synthesize("xin chào", &config()),
            )
            .await;
            assert!(
                result.is_ok(),
                "abrupt WebSocket transport EOF did not terminate synthesize before deadline"
            );
            assert!(
                result.expect("deadline already checked").is_err(),
                "abrupt WebSocket transport EOF must surface as an error"
            );
        });
    }
}
