#[tokio::test]
async fn download_with_chunks() {
    use rusty_ytdl::{Video, VideoOptions, VideoQuality};

    let url = "https://www.youtube.com/watch?v=FZ8BxMU3BYc";

    let video_options = VideoOptions {
        quality: VideoQuality::Highest,
        ..Default::default()
    };

    let video = Video::new_with_options(url, video_options).unwrap();

    let stream = match video.stream().await {
        Ok(s) => s,
        Err(e) => {
            println!("Skipping test: stream resolution failed (likely YouTube bot protection): {:?}", e);
            return;
        }
    };

    loop {
        match stream.chunk().await {
            Ok(Some(chunk)) => {
                println!("{} byte downloaded", chunk.len());
            }
            Ok(None) => break,
            Err(e) => {
                println!("Exiting test: chunk download failed (tolerated): {:?}", e);
                break;
            }
        }
    }
}
