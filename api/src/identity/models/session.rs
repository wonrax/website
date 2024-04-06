use std::ops::Add;

use base64::Engine;
use rand::RngCore;

use crate::crypto::random;

pub struct Session {
    pub id: i32,
    pub active: bool,
    pub token: String,
    pub issued_at: chrono::NaiveDateTime,
    pub expires_at: chrono::NaiveDateTime,
    pub identity_id: i32,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

impl Session {
    /// TODO this function should be ran inside spawn_blocking
    pub fn new_with_identity_id(identity_id: i32) -> Self {
        let mut session_bytes = [0u8; 96];
        random::get_rng().fill_bytes(&mut session_bytes);

        let token =
            "wnrx_".to_owned() + &base64::engine::general_purpose::STANDARD.encode(session_bytes);

        let now = chrono::Utc::now().naive_utc();

        Session {
            id: 0,
            active: true,
            token,
            issued_at: now,
            expires_at: now.add(chrono::Duration::try_days(365).unwrap_or_else(|| {
                tracing::error!("Could not convert 365 to days, using default");
                chrono::Duration::default()
            })),
            identity_id,
            created_at: now,
            updated_at: now,
        }
    }
}
