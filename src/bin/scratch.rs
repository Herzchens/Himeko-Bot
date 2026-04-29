use msedge_tts::tts::client::connect_async;
use msedge_tts::tts::SpeechConfig;
use std::fs::File;
use std::io::Write;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut client = connect_async().await?;
    
    // Normal
    let config_normal = SpeechConfig {
        voice_name: "vi-VN-HoaiMyNeural".to_string(),
        audio_format: "audio-24khz-48kbitrate-mono-mp3".to_string(),
        pitch: 0,
        rate: 0,
        volume: 0,
    };
    let audio = client.synthesize("Xin chào, đây là tốc độ bình thường", &config_normal).await?;
    std::fs::write("normal.mp3", audio.audio_bytes)?;
    
    // Fast + High Pitch
    let config_fast = SpeechConfig {
        voice_name: "vi-VN-HoaiMyNeural".to_string(),
        audio_format: "audio-24khz-48kbitrate-mono-mp3".to_string(),
        pitch: 50,
        rate: 50,
        volume: 0,
    };
    let mut client2 = connect_async().await?;
    let audio2 = client2.synthesize("Xin chào, đây là tốc độ cực kỳ nhanh và cao", &config_fast).await?;
    std::fs::write("fast.mp3", audio2.audio_bytes)?;

    println!("Done!");
    Ok(())
}
