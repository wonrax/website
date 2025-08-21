CREATE UNIQUE INDEX identity_credentials_oauth_unique
ON identity_credentials (identity_id, (credential -> 'provider'))
WHERE
    (credential -> 'provider') NOTNULL;
