use diesel::prelude::*;

#[derive(Queryable, Selectable, Insertable, AsChangeset, Debug)]
#[diesel(table_name = crate::schema::identity_credentials)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct IdentityCredential {
    pub id: i32,
    pub credential: Option<serde_json::Value>,
    pub credential_type_id: i32,
    pub identity_id: i32,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::identity_credentials)]
pub struct NewIdentityCredential {
    pub credential: Option<serde_json::Value>,
    pub credential_type_id: i32,
    pub identity_id: i32,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

impl IdentityCredential {
    pub fn new_oauth_credential(oauth_credential: serde_json::Value) -> NewIdentityCredential {
        let now = chrono::Utc::now().naive_utc();
        NewIdentityCredential {
            credential: Some(oauth_credential),
            credential_type_id: 1, // OAuth type ID from database
            identity_id: 0,        // This will be set when inserting
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone)]
pub enum CredentialType {
    OAuth,
}

impl From<CredentialType> for &'static str {
    fn from(value: CredentialType) -> Self {
        match value {
            CredentialType::OAuth => "oauth",
        }
    }
}

impl<'de> serde::Deserialize<'de> for CredentialType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "oauth" => Ok(CredentialType::OAuth),
            _ => Err(serde::de::Error::custom("invalid credential type")),
        }
    }
}

#[derive(Queryable, Selectable, Debug)]
#[diesel(table_name = crate::schema::identity_credential_types)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[allow(dead_code)]
pub struct IdentityCredentialType {
    pub id: i32,
    pub name: String,
    pub created_at: chrono::NaiveDateTime,
}
