//! jarvis-license-server <command>
//!   init                     create the database, the standard tariffs and (once) the two secret files
//!   public-key               print the public key that goes into the build of Jarvis User
//!   admin-create <name>      create an administrator and print his token ONCE
//!   serve                    run the server (behind a reverse proxy with TLS)
//!   backup <file>            a consistent copy of the database into a NEW file
use jarvis_license_server::{api::Api, config, http, store::Store};
use std::sync::Arc;

fn usage() -> ! {
    eprintln!("использование: jarvis-license-server init | public-key | admin-create <имя> | serve | backup <файл>\nнастройки: см. deploy/license-server.env.example");
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
