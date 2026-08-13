pub fn install_crypto_provider() -> anyhow::Result<()> {
    if rustls::crypto::CryptoProvider::get_default().is_some() {
        return Ok(());
    }

    match rustls::crypto::ring::default_provider().install_default() {
        Ok(()) => Ok(()),
        Err(_) if rustls::crypto::CryptoProvider::get_default().is_some() => Ok(()),
        Err(_) => anyhow::bail!("failed to install rustls ring crypto provider"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_installation_is_idempotent_and_reqwest_client_builds() {
        install_crypto_provider().expect("first provider installation must succeed");
        install_crypto_provider().expect("second provider installation must be idempotent");
        reqwest::Client::builder()
            .build()
            .expect("reqwest rustls client must build after provider installation");
    }
}
