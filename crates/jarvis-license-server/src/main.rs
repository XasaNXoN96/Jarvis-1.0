//! jarvis-license-server <command>
//!   init                     create the database, the standard tariffs and (once) the two secret files
//!   public-key               print the public key that goes into the build of Jarvis User
//!   admin-create <name>      create an administrator and print his token ONCE
//!   serve                    run the server (behind a reverse proxy with TLS)
//!   backup <file>            a consistent copy of the database into a NEW file
//!   update-keygen <file>     a NEW key pair for signing program updates (the secret goes into <file>; the public key is printed)
//!   update-sign <keyfile> <installer.exe> <version> <https-url> [notes]   prints the signed update manifest (update.json)
use jarvis_license_server::{api::Api, config, http, store::Store};
use std::sync::Arc;

fn usage() -> ! {
    eprintln!("использование: jarvis-license-server init | public-key | admin-create <имя> | serve | backup <файл> | update-keygen <файл> | update-sign <ключ> <установщик> <версия> <https-адрес> [заметки]\nнастройки: см. deploy/license-server.env.example");
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().map(|s| s.as_str()) else { usage() };
    if let Err(e) = run(cmd, &args[1..]) {
        eprintln!("ошибка: {}", e);
        std::process::exit(1);
    }
}

fn run(cmd: &str, rest: &[String]) -> Result<(), String> {
    match cmd {
        "init" => {
            let signing = std::env::var("JARVIS_LS_SIGNING_KEY_FILE").map_err(|_| "задайте JARVIS_LS_SIGNING_KEY_FILE")?;
            let pepper = std::env::var("JARVIS_LS_PEPPER_FILE").map_err(|_| "задайте JARVIS_LS_PEPPER_FILE")?;
            let created = config::init_secrets(std::path::Path::new(&signing), std::path::Path::new(&pepper))?;
            let cfg = config::Config::from_env()?;
            let store = Store::open(&cfg.db_path, cfg.pepper.clone(), cfg.signing.clone(), cfg.grace_hours, cfg.default_activation_days)?;
            println!("база готова (схема v{}), стандартные тарифы созданы только если таблица была пуста", store.schema_version());
            if created {
                println!("созданы файлы секретов (сделайте их резервную копию и держите вне репозитория)");
            }
            println!("публичный ключ для сборки Jarvis User: {}", public_key_b64(&cfg));
            Ok(())
        }
        "public-key" => {
            let cfg = config::Config::from_env()?;
            println!("{}", public_key_b64(&cfg));
            Ok(())
        }
        "admin-create" => {
            let name = rest.first().ok_or("укажите имя администратора")?;
            let cfg = config::Config::from_env()?;
            let store = Store::open(&cfg.db_path, cfg.pepper.clone(), cfg.signing.clone(), cfg.grace_hours, cfg.default_activation_days)?;
            let token = store.admin_create(name, http::now())?;
            println!("токен администратора «{}» (показан один раз, сервер хранит только хеш):\n{}", name, token);
            Ok(())
        }
        "backup" => {
            let target = rest.first().ok_or("укажите файл копии")?;
            let cfg = config::Config::from_env()?;
            let store = Store::open(&cfg.db_path, cfg.pepper.clone(), cfg.signing.clone(), cfg.grace_hours, cfg.default_activation_days)?;
            store.backup_to(std::path::Path::new(target))?;
            println!("копия базы: {}", target);
            Ok(())
        }
        "update-keygen" => {
            use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
            let file = std::path::Path::new(rest.first().ok_or("укажите файл для секретного ключа обновлений")?);
            if file.exists() {
                return Err("файл уже есть: ключ не перезаписывается".into());
            }
            let sk = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
            // the secret file has the same form as the other secret files of the server (standard base64), the public key the form of the build
            use base64::engine::general_purpose::STANDARD as B64S;
            config::write_secret_file(file, &B64S.encode(sk.to_bytes())).map_err(|e| e.to_string())?;
            println!("секретный ключ обновлений записан в {} (храните вне репозитория и вне сервера, сделайте резервную копию)", file.display());
            println!("публичный ключ для сборки (JARVIS_UPDATE_PUBKEY): {}", B64.encode(sk.verifying_key().to_bytes()));
            Ok(())
        }
        "update-sign" => {
            let [key_file, installer, version, url] = [0, 1, 2, 3].map(|i| rest.get(i).cloned());
            let (Some(key_file), Some(installer), Some(version), Some(url)) = (key_file, installer, version, url) else {
                return Err("нужно: update-sign <ключ> <установщик> <версия> <https-адрес> [заметки]".into());
            };
            if !url.starts_with("https://") {
                return Err("адрес установщика должен начинаться с https://".into());
            }
            let sk = config::load_signing_key(std::path::Path::new(&key_file))?;
            let bytes = std::fs::read(&installer).map_err(|e| format!("не читается {}: {}", installer, e))?;
            let sha = jarvis_license::sha256_hex(&bytes);
            let sig = jarvis_license::sign_update(&sk, &version, &url, &sha, bytes.len() as u64);
            let notes = rest.get(4..).map(|r| r.join(" ")).unwrap_or_default();
            let manifest = serde_json::json!({ "version": version, "url": url, "sha256": sha, "size": bytes.len(), "notes": notes, "sig": sig });
            println!("{}", serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?);
            Ok(())
        }
        "serve" => {
            let cfg = config::Config::from_env()?;
            let store = Store::open(&cfg.db_path, cfg.pepper.clone(), cfg.signing.clone(), cfg.grace_hours, cfg.default_activation_days)?;
            let api = Arc::new(Api::new(store, cfg.renew_info.clone()));
            println!("Jarvis License Server слушает {} (TLS — на обратном прокси)", cfg.bind);
            http::run(api, &cfg.bind, cfg.client_ip_header.clone())
        }
        _ => usage(),
    }
}

fn public_key_b64(cfg: &config::Config) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
    B64.encode(cfg.signing.verifying_key().to_bytes())
}
