use serde_json::Value as JsonValue;

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

#[derive(Debug)]
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

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Traits {
    pub email: Option<String>,
    pub name: Option<String>,
}

impl Traits {
    /// Defines rules for determining which fields are identifiers. Returns a
    /// list of references to the fields that are considered identifiers.
    pub fn get_identifiers(&self) -> Vec<&String> {
        let mut ids = vec![];
        if let Some(email) = &self.email {
            ids.push(email);
        }
        ids
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
