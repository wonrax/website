//! When each channel's sandbox was last used, which decides when it's deleted

use std::num::NonZeroU64;

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use eyre::Context as _;
use serenity::all::ChannelId;

use crate::{discord::chatgpt::DbPool, schema::discord_sandboxes};

#[derive(Clone)]
pub struct SandboxStore {
    db: DbPool,
}

impl SandboxStore {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    /// Records that the sandbox of `channel_id` is in use as of now
    pub async fn touch(&self, channel_id: ChannelId) -> eyre::Result<()> {
        let mut conn = self.db.get().await.context("No database connection")?;
        diesel::insert_into(discord_sandboxes::table)
            .values((
                discord_sandboxes::channel_id.eq(channel_id.get().cast_signed()),
                discord_sandboxes::last_used_at.eq(Utc::now()),
            ))
            .on_conflict(discord_sandboxes::channel_id)
            .do_update()
            .set(discord_sandboxes::last_used_at.eq(Utc::now()))
            .execute(&mut conn)
            .await
            .context("Failed to record when the sandbox was used")?;
        Ok(())
    }

    /// Starts keeping time for a sandbox the table doesn't know, as though it were used just now
    pub async fn adopt(&self, channel_id: ChannelId) -> eyre::Result<()> {
        let mut conn = self.db.get().await.context("No database connection")?;
        diesel::insert_into(discord_sandboxes::table)
            .values((
                discord_sandboxes::channel_id.eq(channel_id.get().cast_signed()),
                discord_sandboxes::last_used_at.eq(Utc::now()),
            ))
            .on_conflict_do_nothing()
            .execute(&mut conn)
            .await
            .context("Failed to record a sandbox")?;
        Ok(())
    }

    /// The channels whose sandbox has gone unused since `cutoff`
    pub async fn unused_since(&self, cutoff: DateTime<Utc>) -> eyre::Result<Vec<ChannelId>> {
        let mut conn = self.db.get().await.context("No database connection")?;
        let ids: Vec<i64> = discord_sandboxes::table
            .filter(discord_sandboxes::last_used_at.lt(cutoff))
            .select(discord_sandboxes::channel_id)
            .load(&mut conn)
            .await
            .context("Failed to look up the unused sandboxes")?;
        Ok(ids
            .into_iter()
            .filter_map(|id| NonZeroU64::new(id.cast_unsigned()).map(ChannelId::from))
            .collect())
    }

    /// Stops keeping time for a deleted sandbox
    pub async fn forget(&self, channel_id: ChannelId) -> eyre::Result<()> {
        let mut conn = self.db.get().await.context("No database connection")?;
        diesel::delete(discord_sandboxes::table.find(channel_id.get().cast_signed()))
            .execute(&mut conn)
            .await
            .context("Failed to forget a deleted sandbox")?;
        Ok(())
    }
}
