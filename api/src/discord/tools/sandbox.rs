use crate::discord::sandbox::{ChannelSandbox, HOME};
use base64::Engine as _;
use image::{DynamicImage, ImageFormat, ImageReader, imageops::FilterType};
use rig::{
    message::{ImageMediaType, ToolResultContent},
    tool::{PortableTool, ToolOutput},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{io::Cursor, time::Duration};
use thiserror::Error;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 30 * 60;
/// How `timeout` exits after stopping a command at its timeout
const TIMED_OUT: i64 = 124;
/// How a command exits after SIGKILL: from `timeout` when it ignored the stop, or from the kernel
/// when the sandbox ran out of memory
const KILLED: i64 = 137;

/// Largest image file `sandbox_view_image` reads
const MAX_IMAGE_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// Longest side of an image the model is shown
const MAX_IMAGE_SIDE: u32 = 1600;
/// Largest image the model is shown. Tool results ride along for the rest of the session.
const MAX_SHOWN_IMAGE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Error)]
#[error("Sandbox error: {0}")]
pub struct SandboxToolError(String);

#[derive(Debug, Clone)]
pub struct SandboxRunTool {
    pub sandbox: ChannelSandbox,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxRunArgs {
    pub command: String,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SandboxRunOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// What `output` leaves out, or why there's no exit code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl PortableTool for SandboxRunTool {
    const NAME: &'static str = "sandbox_run";
    type Error = SandboxToolError;
    type Args = SandboxRunArgs;
    type Output = SandboxRunOutput;

    fn description(&self) -> String {
        format!(
            "Run a shell command on your own Linux computer: a sandbox for this channel with \
internet access (1 GB of memory, 1.5 CPUs), for cloning public repos, running Python, building \
and testing code, and making files to post with send_discord_message. Each call is a fresh \
non-interactive bash in {HOME}, so cd and exports don't carry over; output is stdout and stderr \
combined.

{HOME} is kept across your sessions until the sandbox goes two weeks unused, so look there, and \
in any notes you left there, before redoing work. Everything outside it resets when the sandbox \
restarts after sitting idle, including whatever was installed.

The machine runs Nix with flakes, and has bash, coreutils, git, curl, python3, ripgrep, and jq \
besides. Get other software from nixpkgs: `nix shell nixpkgs#pkg -c cmd`, or \
`nix-shell -p 'python3.withPackages (ps: [ ps.numpy ])' --run cmd` for Python libraries, since \
pip wheels with native code don't load here. A project gets a flake.nix with a devShell.

A background process has to redirect its output, or the call waits for it until the timeout."
        )
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Bash script to run."
                },
                "timeout_secs": {
                    "type": ["integer", "null"],
                    "description": format!("Seconds before the command is killed: {DEFAULT_TIMEOUT_SECS} when null, at most {MAX_TIMEOUT_SECS}.")
                }
            },
            "required": ["command", "timeout_secs"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let timeout_secs = args
            .timeout_secs
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(1, MAX_TIMEOUT_SECS);
        let run = match self
            .sandbox
            .run(&args.command, Duration::from_secs(timeout_secs))
            .await
        {
            Ok(run) => run,
            Err(e) => {
                tracing::error!(?e, "Sandbox command failed to run");
                return Ok(SandboxRunOutput {
                    error: Some(format!("{e:#}")),
                    ..Default::default()
                });
            }
        };

        let mut notes = Vec::new();
        match run.exit_code {
            Some(TIMED_OUT) => notes.push(format!("Stopped at its {timeout_secs}s timeout.")),
            Some(KILLED) => notes.push(format!(
                "Killed: it reached its {timeout_secs}s timeout, or the sandbox ran out of memory."
            )),
            Some(_) => {}
            None => notes.push(
                "No exit code: its output stayed open past the timeout, likely held by a \
                 background process that may still be running."
                    .to_string(),
            ),
        }
        if run.omitted_bytes > 0 {
            notes.push(format!(
                "The middle {} bytes of the output are cut; all of it is in {}.",
                run.omitted_bytes, run.log_path
            ));
        }
        Ok(SandboxRunOutput {
            exit_code: run.exit_code,
            output: Some(run.output),
            note: (!notes.is_empty()).then(|| notes.join(" ")),
            error: None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SandboxWriteFileTool {
    pub sandbox: ChannelSandbox,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxWriteFileArgs {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SandboxWriteFileOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl PortableTool for SandboxWriteFileTool {
    const NAME: &'static str = "sandbox_write_file";
    type Error = SandboxToolError;
    type Args = SandboxWriteFileArgs;
    type Output = SandboxWriteFileOutput;

    fn description(&self) -> String {
        "Write a text file in your sandbox (see sandbox_run), replacing it if it exists and \
creating missing directories. Use it for code and any text whose quoting a shell command would \
mangle."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": format!("Absolute, or relative to {HOME}.")
                },
                "content": {
                    "type": "string",
                    "description": "The whole file."
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        match self
            .sandbox
            .write_file(&args.path, args.content.as_bytes())
            .await
        {
            Ok(path) => Ok(SandboxWriteFileOutput {
                path: Some(path),
                bytes: Some(args.content.len()),
                error: None,
            }),
            Err(e) => {
                tracing::error!(?e, "Failed to write a sandbox file");
                Ok(SandboxWriteFileOutput {
                    error: Some(format!("{e:#}")),
                    ..Default::default()
                })
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct SandboxViewImageTool {
    pub sandbox: ChannelSandbox,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxViewImageArgs {
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
struct ViewImageResponse {
    path: String,
    /// The size the original has, when the model is shown a smaller copy
    #[serde(skip_serializing_if = "Option::is_none")]
    scaled_down_from: Option<String>,
}

impl PortableTool for SandboxViewImageTool {
    const NAME: &'static str = "sandbox_view_image";
    type Error = SandboxToolError;
    type Args = SandboxViewImageArgs;
    type Output = ToolOutput;

    fn description(&self) -> String {
        "Look at an image file in your sandbox (see sandbox_run), such as a chart you rendered, \
to check it before posting it or to read what it shows. PNG, JPEG, GIF, or WebP; large images are \
scaled down for you."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": format!("Absolute, or relative to {HOME}.")
                }
            },
            "required": ["path"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let failure = |error: String| ToolOutput::text(json!({ "error": error }).to_string());
        let file = match self
            .sandbox
            .read_file(&args.path, MAX_IMAGE_FILE_BYTES)
            .await
        {
            Ok(file) => file,
            Err(e) => return Ok(failure(format!("{e:#}"))),
        };
        let prepared = tokio::task::spawn_blocking(move || prepare_image(file.bytes)).await;
        let image = match prepared {
            Ok(Ok(image)) => image,
            Ok(Err(e)) => return Ok(failure(format!("{}: {e:#}", args.path))),
            Err(e) => return Ok(failure(format!("Failed to read the image: {e}"))),
        };

        let response = ViewImageResponse {
            path: args.path,
            scaled_down_from: image
                .original_size
                .map(|(width, height)| format!("{width}x{height}")),
        };
        let response = match serde_json::to_value(&response) {
            Ok(response) => response,
            Err(e) => return Ok(failure(format!("Failed to describe the image: {e}"))),
        };
        let content = vec![
            ToolResultContent::json(response),
            ToolResultContent::image_base64(
                base64::prelude::BASE64_STANDARD.encode(&image.bytes),
                Some(image.media_type),
                None,
            ),
        ];
        Ok(ToolOutput::content(content).unwrap_or_else(|e| ToolOutput::text(e.to_string())))
    }
}

/// An image as the model is shown it
struct ShownImage {
    bytes: Vec<u8>,
    media_type: ImageMediaType,
    /// The original's width and height, when it had to be scaled down
    original_size: Option<(u32, u32)>,
}

/// `bytes` as the model can be shown them: as they are when small enough, otherwise scaled down
/// to `MAX_IMAGE_SIDE` and re-encoded
fn prepare_image(bytes: Vec<u8>) -> eyre::Result<ShownImage> {
    let format = ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()?
        .format()
        .ok_or_else(|| eyre::eyre!("not a PNG, JPEG, GIF, or WebP image"))?;
    let media_type = match format {
        ImageFormat::Png => Some(ImageMediaType::PNG),
        ImageFormat::Jpeg => Some(ImageMediaType::JPEG),
        ImageFormat::Gif => Some(ImageMediaType::GIF),
        ImageFormat::WebP => Some(ImageMediaType::WEBP),
        _ => None,
    };
    let (width, height) = ImageReader::with_format(Cursor::new(&bytes), format)
        .into_dimensions()
        .map_err(|e| eyre::eyre!("not an image this tool can read ({e})"))?;
    let fits = width.max(height) <= MAX_IMAGE_SIDE;
    if let Some(media_type) = media_type
        && fits
        && bytes.len() <= MAX_SHOWN_IMAGE_BYTES
    {
        return Ok(ShownImage {
            bytes,
            media_type,
            original_size: None,
        });
    }

    let image = ImageReader::with_format(Cursor::new(&bytes), format)
        .decode()
        .map_err(|e| eyre::eyre!("couldn't decode the image ({e})"))?;
    let (image, original_size) = if fits {
        (image, None)
    } else {
        (
            image.resize(MAX_IMAGE_SIDE, MAX_IMAGE_SIDE, FilterType::Triangle),
            Some((width, height)),
        )
    };
    let png = encode(&image, ImageFormat::Png)?;
    if png.len() <= MAX_SHOWN_IMAGE_BYTES {
        return Ok(ShownImage {
            bytes: png,
            media_type: ImageMediaType::PNG,
            original_size,
        });
    }
    // A photo, most likely, which PNG keeps too big
    let jpeg = encode(&DynamicImage::ImageRgb8(image.to_rgb8()), ImageFormat::Jpeg)?;
    Ok(ShownImage {
        bytes: jpeg,
        media_type: ImageMediaType::JPEG,
        original_size,
    })
}

fn encode(image: &DynamicImage, format: ImageFormat) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), format)
        .map_err(|e| eyre::eyre!("couldn't re-encode the image ({e})"))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = DynamicImage::new_rgb8(width, height);
        encode(&image, ImageFormat::Png).expect("encode a test PNG")
    }

    #[test]
    fn small_images_pass_through() {
        let bytes = png(40, 30);
        let shown = prepare_image(bytes.clone()).expect("a small PNG");
        assert_eq!(shown.bytes, bytes);
        assert!(matches!(shown.media_type, ImageMediaType::PNG));
        assert_eq!(shown.original_size, None);
    }

    #[test]
    fn large_images_are_scaled_down() {
        let shown = prepare_image(png(4000, 1000)).expect("a large PNG");
        assert_eq!(shown.original_size, Some((4000, 1000)));
        let reader = ImageReader::new(Cursor::new(&shown.bytes))
            .with_guessed_format()
            .expect("read the scaled image");
        assert_eq!(reader.into_dimensions().ok(), Some((1600, 400)));
    }

    #[test]
    fn non_images_are_refused() {
        assert!(prepare_image(b"#!/bin/sh\necho hi\n".to_vec()).is_err());
    }
}
