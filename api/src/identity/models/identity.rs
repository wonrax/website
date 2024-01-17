use std::collections::HashMap;

use serde_json::Value as JsonValue;
use sqlx::Encode;

use super::credential::{CredentialType, IdentityCredential};

pub enum IdentityState {
    Active,
    Inactive,
    Locked,
}

impl IdentityState {
    pub fn as_str(&self) -> &'static str {
        match self {
            IdentityState::Active => "active",
            IdentityState::Inactive => "inactive",
            IdentityState::Locked => "locked",
        }
    }
}

pub struct Identity {
    pub id: i32,
    pub traits: Traits,

    // Optional means will be expanded (filled) when needed (e.g. from database)
    // pub credentials: Option<HashMap<CredentialType, IdentityCredential>>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

impl Identity {
    pub fn new_with_traits(traits: Traits) -> Self {
        let now = chrono::Utc::now().naive_utc();

        Identity {
            id: 0,
            traits,
            // credentials: None,
            created_at: now,
            updated_at: now,
        }
    }

    // pub fn with_credentials(
    //     mut self,
    //     credentials: HashMap<CredentialType, IdentityCredential>,
    // ) -> Self {
    //     self.credentials = Some(credentials);
    //     self
    // }
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Traits {
    pub email: String,
    pub name: Option<String>,
}

impl Traits {
    /// Defines rules for determining which fields are identifiers.
    pub fn get_identifiers(&self) -> Vec<&String> {
        vec![&self.email]
    }
}

// TODO maybe write a macro for this?
impl From<&Traits> for JsonValue {
    fn from(t: &Traits) -> Self {
        serde_json::to_value(t).unwrap()
    }
}

impl From<JsonValue> for Traits {
    fn from(value: JsonValue) -> Self {
        serde_json::from_value(value).unwrap()
    }
}
