use crate::discord::message::{download_attachment, image_media_type, parse_message_id};
use base64::Engine as _;
use rig::{completion::ToolDefinition, message::MimeType as _, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serenity::all::{ChannelId, Context, MessageId};
use std::sync::Arc;
use thiserror::Error;

/// Largest single image the tool will load; Discord memes and screenshots are well under this
const MAX_IMAGE_BYTES: u32 = 4 * 1024 * 1024;
/// Cap on the images loaded by one call. Tool results are resent with every model turn for the
/// rest of the session, so this bounds the request size.
const MAX_TOTAL_BYTES: u32 = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ViewMessageAttachmentsTool {
    pub ctx: Arc<Context>,
    pub channel_id: ChannelId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewMessageAttachmentsArgs {
    pub message_id: String,
}

/// rig's hybrid tool output: `response` becomes the text part of the tool result and each entry
/// of `parts` an image part, so the model sees the pixels rather than a base64 string.
#[derive(Debug, Clone, Serialize)]
pub struct ViewMessageAttachmentsOutput {
    pub response: AttachmentsResponse,
    pub parts: Vec<ImagePart>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttachmentsResponse {
    pub success: bool,
    pub message_id: String,
    /// Filenames of the images attached to this result, in order
    pub shown: Vec<String>,
    pub skipped: Vec<SkippedAttachment>,
    pub note: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkippedAttachment {
    pub filename: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImagePart {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub data: String,
    #[serde(rename = "mimeType")]
    pub mime_type: &'static str,
}

impl ViewMessageAttachmentsOutput {
    fn text_only(
        message_id: String,
        success: bool,
        note: Option<String>,
        error: Option<String>,
    ) -> Self {
        Self {
            response: AttachmentsResponse {
                success,
                message_id,
                shown: vec![],
                skipped: vec![],
                note,
                error,
            },
            parts: vec![],
        }
    }

    fn failure(message_id: String, error: String) -> Self {
        Self::text_only(message_id, false, None, Some(error))
    }
}

#[derive(Debug, Error)]
#[error("View message attachments error: {0}")]
pub struct ViewMessageAttachmentsError(String);

impl Tool for ViewMessageAttachmentsTool {
    const NAME: &'static str = "view_message_attachments";
    type Error = ViewMessageAttachmentsError;
    type Args = ViewMessageAttachmentsArgs;
    type Output = ViewMessageAttachmentsOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Show yourself the image attachments on a message. Attachments in your starting context and in fetched history appear by name only, like [Attachment: cat.png]; call this with the ID from that message's [#ID] header to see them. Images on live messages are already shown to you inline, so those don't need fetching. Non-image attachments are reported by name but cannot be shown."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "message_id": {
                        "type": "string",
                        "description": "The ID from the [#ID] header of the message whose attachments you want to see."
                    }
                },
                "required": ["message_id"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let message_id = match parse_message_id("message_id", &args.message_id) {
            Ok(id) => id.get(),
            Err(error) => {
                return Ok(ViewMessageAttachmentsOutput::failure(
                    args.message_id.clone(),
                    error,
                ));
            }
        };

        let ctx = self.ctx.clone();
        let channel_id = self.channel_id;

        // Spawn the Discord API operations in a separate task to avoid Sync issues
        let handle = tokio::spawn(async move {
            let message = match channel_id
                .message(&ctx.http, MessageId::new(message_id))
                .await
            {
                Ok(message) => message,
                Err(e) => {
                    tracing::error!(?e, message_id, "Failed to fetch message for attachments");
                    return ViewMessageAttachmentsOutput::failure(
                        message_id.to_string(),
                        format!(
                            "could not fetch message {message_id}: {e}; it may have been deleted"
                        ),
                    );
                }
            };

            if message.attachments.is_empty() {
                return ViewMessageAttachmentsOutput::text_only(
                    message_id.to_string(),
                    true,
                    Some("this message has no attachments".to_string()),
                    None,
                );
            }

            let mut shown = Vec::new();
            let mut skipped = Vec::new();
            let mut parts = Vec::new();
            let mut budget = MAX_TOTAL_BYTES;

            for attachment in &message.attachments {
                let filename = attachment.filename.clone();

                let Some(media_type) = image_media_type(attachment) else {
                    skipped.push(SkippedAttachment {
                        filename,
                        reason: format!(
                            "not an image ({})",
                            attachment.content_type.as_deref().unwrap_or("unknown type")
                        ),
                    });
                    continue;
                };

                if attachment.size > MAX_IMAGE_BYTES {
                    skipped.push(SkippedAttachment {
                        filename,
                        reason: format!(
                            "{} bytes is over the {} byte limit for one image",
                            attachment.size, MAX_IMAGE_BYTES
                        ),
                    });
                    continue;
                }

                if attachment.size > budget {
                    skipped.push(SkippedAttachment {
                        filename,
                        reason: "the images before it used up this call's size budget".to_string(),
                    });
                    continue;
                }

                match download_attachment(attachment).await {
                    Ok(bytes) => {
                        budget =
                            budget.saturating_sub(u32::try_from(bytes.len()).unwrap_or(u32::MAX));
                        parts.push(ImagePart {
                            kind: "image",
                            data: base64::prelude::BASE64_STANDARD.encode(&bytes),
                            mime_type: media_type.to_mime_type(),
                        });
                        shown.push(filename);
                    }
                    Err(error) => {
                        tracing::error!(?error, filename = %attachment.filename, "Failed to download attachment");
                        skipped.push(SkippedAttachment {
                            filename,
                            reason: format!("download failed: {error}"),
                        });
                    }
                }
            }

            tracing::debug!(
                message_id,
                shown = shown.len(),
                skipped = skipped.len(),
                "view_message_attachments completed"
            );

            ViewMessageAttachmentsOutput {
                response: AttachmentsResponse {
                    success: true,
                    message_id: message_id.to_string(),
                    shown,
                    skipped,
                    note: None,
                    error: None,
                },
                parts,
            }
        });

        match handle.await {
            Ok(output) => Ok(output),
            Err(e) => {
                tracing::error!(?e, "Task join error while viewing message attachments");
                Ok(ViewMessageAttachmentsOutput::failure(
                    message_id.to_string(),
                    format!("Task execution failed: {e}"),
                ))
            }
        }
    }
}
