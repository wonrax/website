pub struct IdentityCredential {
    pub id: i32,

    // Credential contains data that can be used to authenticate (e.g. password
    // hash if the credential type is password).
    pub credential: serde_json::value::Value,
    pub credential_type: CredentialType,

    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

impl IdentityCredential {
    pub fn new_oidc_credential(oidc_credential: serde_json::Value) -> Self {
        let now = chrono::Utc::now().naive_utc();

        IdentityCredential {
            id: 0,
            credential: oidc_credential,
            credential_type: CredentialType::Oidc,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone)]
pub enum CredentialType {
    Oidc,
}

impl<'de> serde::Deserialize<'de> for CredentialType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "oidc" => Ok(CredentialType::Oidc),
            _ => Err(serde::de::Error::custom("invalid credential type")),
        }
    }
}
