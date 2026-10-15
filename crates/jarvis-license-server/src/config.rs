//! Configuration: the environment and the files of the secrets. No secret has a default value and none is in the repository.
use base64::{engine::general_purpose::STANDARD as B64S, Engine};
use ed25519_dalek::SigningKey;
use std::path::{Path, PathBuf};

pub struct Config {
    pub db_path: PathBuf,
    pub bind: String,
    // the signing key: ONLY here, never in the program of the customer
    pub signing: SigningKey,
    // the secret that goes into the hash of the keys and of the admin tokens: outside of the database
    pub pepper: Vec<u8>,
    pub grace_hours: i64,
    // how long a new key may wait for its first activation when the administrator did not say (days)
    pub default_activation_days: i64,
    // the text shown to the customer who wants to buy or to renew (no payment system is connected)
    pub renew_info: String,
    // behind a reverse proxy the address of the customer is in this header (for the rate limits); empty = the socket address
    pub client_ip_header: String,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

pub fn write_secret_file(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn load_signing_key(path: &Path) -> Result<SigningKey, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("не читается ключ подписи {}: {}", path.display(), e))?;
    let seed: [u8; 32] = B64S.decode(text.trim()).ok().and_then(|b| b.try_into().ok()).ok_or("ключ подписи повреждён (нужны 32 байта в base64)")?;
    Ok(SigningKey::from_bytes(&seed))
}

pub fn load_pepper(path: &Path) -> Result<Vec<u8>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("не читается pepper {}: {}", path.display(), e))?;
    let bytes = B64S.decode(text.trim()).map_err(|_| "pepper повреждён (base64)".to_string())?;
    if bytes.len() < 32 {
        return Err("pepper короче 32 байт".into());
    }
    Ok(bytes)
}

// creates the two secret files once (never overwrites): returns true when something was created
pub fn init_secrets(signing_path: &Path, pepper_path: &Path) -> Result<bool, String> {
    use rand::RngCore;
    let mut created = false;
    if !signing_path.exists() {
        let sk = SigningKey::generate(&mut rand::rngs::OsRng);
        write_secret_file(signing_path, &B64S.encode(sk.to_bytes())).map_err(|e| e.to_string())?;
        created = true;
    }
    if !pepper_path.exists() {
        let mut p = [0u8; 48];
        rand::rngs::OsRng.fill_bytes(&mut p);
        write_secret_file(pepper_path, &B64S.encode(p)).map_err(|e| e.to_string())?;
        created = true;
    }
    Ok(created)
}

impl Config {
    // everything comes from the environment (see deploy/license-server.env.example)
    pub fn from_env() -> Result<Config, String> {
        let signing_path = PathBuf::from(env("JARVIS_LS_SIGNING_KEY_FILE").ok_or("задайте JARVIS_LS_SIGNING_KEY_FILE")?);
        let pepper_path = PathBuf::from(env("JARVIS_LS_PEPPER_FILE").ok_or("задайте JARVIS_LS_PEPPER_FILE")?);
        Ok(Config {
            db_path: PathBuf::from(env("JARVIS_LS_DB").ok_or("задайте JARVIS_LS_DB")?),
            bind: env("JARVIS_LS_BIND").unwrap_or_else(|| "127.0.0.1:8787".into()),
            signing: load_signing_key(&signing_path)?,
            pepper: load_pepper(&pepper_path)?,
            grace_hours: env("JARVIS_LS_GRACE_HOURS").and_then(|v| v.parse().ok()).unwrap_or(72).clamp(1, 24 * 30),
            default_activation_days: env("JARVIS_LS_ACTIVATION_DAYS").and_then(|v| v.parse().ok()).unwrap_or(365).clamp(1, 3650),
            renew_info: env("JARVIS_LS_RENEW_INFO").unwrap_or_else(|| "Чтобы продлить Jarvis Premium, получите новый ключ у владельца продукта и введите его в этом окне.".into()),
            client_ip_header: env("JARVIS_LS_CLIENT_IP_HEADER").unwrap_or_default(),
        })
    }
}
