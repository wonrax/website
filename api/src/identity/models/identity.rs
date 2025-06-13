use diesel::prelude::*;
use serde_json::Value as JsonValue;

#[allow(dead_code)]
pub enum IdentityState {
    Active,
    Inactive,
    Locked,
}

impl IdentityState {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            IdentityState::Active => "active",
            IdentityState::Inactive => "inactive",
            IdentityState::Locked => "locked",
        }
    }
}

#[derive(Queryable, Selectable, Insertable, AsChangeset, Debug)]
#[diesel(table_name = crate::schema::identities)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Identity {
    pub id: i32,
    pub traits: JsonValue,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::identities)]
pub struct NewIdentity {
    pub traits: JsonValue,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

impl Identity {
    pub fn new_with_traits(traits: Traits) -> NewIdentity {
        let now = chrono::Utc::now().naive_utc();
        NewIdentity {
            traits: JsonValue::from(&traits),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn get_traits(&self) -> Traits {
        Traits::from(self.traits.clone())
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
