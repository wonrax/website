UPDATE identity_credentials
SET credential = credential - 'oidc_provider'
                 || jsonb_build_object('provider', credential->'oidc_provider')
WHERE credential ? 'oidc_provider';
