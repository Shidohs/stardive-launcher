use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use uuid::Uuid;

const NM_API_BASE: &str = "https://apis.netmarble.com/cpplauncher/api/external";
const NM_LAUNCHER_CH: &str = "ypWjRL2aNi";
const NM_LAUNCHER_VER: &str = "1.7.0";
const GAME_CODE: &str = "monster2";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    pub launcher_token: String,
    pub channel: String,
    pub channel_code: u32,
    pub player_id: String,
    pub profile_name: String,
    pub profile_img_url: String,
    pub expires_at: i64,
    pub device_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStatusPayload {
    pub is_logged_in: bool,
    pub channel: String,
    pub player_id: String,
    pub profile_name: String,
    pub profile_img_url: String,
    pub expires_at: i64,
}

impl Default for AuthStatusPayload {
    fn default() -> Self {
        Self {
            is_logged_in: false,
            channel: String::new(),
            player_id: String::new(),
            profile_name: String::new(),
            profile_img_url: String::new(),
            expires_at: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelPayload {
    pub channel: String,
    #[serde(rename = "idToken", default)]
    pub id_token: Option<String>,
    #[serde(rename = "googleClientId", default)]
    pub google_client_id: Option<String>,
    #[serde(rename = "accessToken", default)]
    pub access_token: Option<String>,
    #[serde(rename = "user", default)]
    pub user: Option<String>,
    #[serde(rename = "code", default)]
    pub code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NetmarbleApiResponse<T> {
    pub code: i32,
    #[serde(default)]
    pub msg: Option<String>,
    #[serde(default)]
    pub data: Option<T>,
}

#[derive(Debug, Default, Deserialize)]
struct GameProfileData {
    #[allow(dead_code)]
    #[serde(default, rename = "playerId")]
    pub player_id: Option<String>,
    #[serde(default, rename = "profileName")]
    pub profile_name: Option<String>,
    #[serde(default, rename = "profileImgURL")]
    pub profile_img_url: Option<String>,
}

pub struct AuthManager;

impl AuthManager {
    pub fn auth_file_path() -> PathBuf {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/home/shidox"))
                    .join(".config")
            })
            .join("stardive-launcher");
        let _ = fs::create_dir_all(&config_dir);
        config_dir.join("auth.json")
    }

    pub fn get_or_create_device_key() -> String {
        let path = dirs::config_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/home/shidox"))
                    .join(".config")
            })
            .join("stardive-launcher")
            .join("device_key");

        if let Ok(key) = fs::read_to_string(&path) {
            let trimmed = key.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }

        let new_key = Uuid::new_v4().to_string();
        let _ = fs::write(&path, &new_key);
        new_key
    }

    pub fn load_session() -> Result<Option<AuthSession>> {
        let path = Self::auth_file_path();
        if !path.exists() {
            return Ok(None);
        }

        let data = fs::read_to_string(&path)?;
        match serde_json::from_str::<AuthSession>(&data) {
            Ok(session) => {
                let now = chrono::Utc::now().timestamp();
                if session.expires_at > 0 && session.expires_at < now {
                    eprintln!("Sesión de Netmarble expirada. Se requiere iniciar sesión nuevamente.");
                    let _ = fs::remove_file(&path);
                    return Ok(None);
                }
                Ok(Some(session))
            }
            Err(e) => {
                eprintln!("Error al leer auth.json: {}", e);
                Ok(None)
            }
        }
    }

    pub fn save_session(session: &AuthSession) -> Result<()> {
        let path = Self::auth_file_path();
        let json = serde_json::to_string_pretty(session)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn clear_session() -> Result<()> {
        let path = Self::auth_file_path();
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    pub fn get_status() -> AuthStatusPayload {
        if let Ok(Some(session)) = Self::load_session() {
            AuthStatusPayload {
                is_logged_in: true,
                channel: session.channel,
                player_id: session.player_id,
                profile_name: session.profile_name,
                profile_img_url: session.profile_img_url,
                expires_at: session.expires_at,
            }
        } else {
            AuthStatusPayload::default()
        }
    }

    /// Canjea los tokens del proveedor (Google, Email, Apple) por el token del launcher (JWT)
    pub async fn exchange_channel_token(
        payload: &ChannelPayload,
        device_key: &str,
    ) -> Result<AuthSession> {
        let client = reqwest::Client::builder().build()?;
        let url = format!("{}/cpp-auth/game/{}/token", NM_API_BASE, GAME_CODE);

        let (channel_code, req_body) = match payload.channel.as_str() {
            "google" => {
                let id_token = payload
                    .id_token
                    .as_deref()
                    .ok_or_else(|| anyhow!("idToken faltante para Google"))?;
                let client_id = payload.google_client_id.as_deref().unwrap_or_default();
                (
                    29u32,
                    serde_json::json!({
                        "deviceKey": device_key,
                        "channelCode": 29,
                        "googleIdToken": id_token,
                        "googleClientId": client_id,
                        "googlePlayerId": ""
                    }),
                )
            }
            "email" => {
                let access_token = payload
                    .access_token
                    .as_deref()
                    .ok_or_else(|| anyhow!("accessToken faltante para Correo Netmarble"))?;
                (
                    20u32,
                    serde_json::json!({
                        "deviceKey": device_key,
                        "channelCode": 20,
                        "channelAccessToken": access_token
                    }),
                )
            }
            "apple" => {
                let id_token = payload
                    .id_token
                    .as_deref()
                    .ok_or_else(|| anyhow!("idToken faltante para Apple"))?;
                (
                    26u32,
                    serde_json::json!({
                        "deviceKey": device_key,
                        "channelCode": 26,
                        "user": payload.user.as_deref().unwrap_or_default(),
                        "authorizationCode": payload.code.as_deref().unwrap_or_default(),
                        "identityToken": id_token
                    }),
                )
            }
            other => return Err(anyhow!("Canal de autenticación desconocido: {}", other)),
        };

        println!("Canjeando token con Netmarble Auth (Canal: {})...", payload.channel);

        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("X-NM-LAUNCHER-CH", NM_LAUNCHER_CH)
            .header("X-NM-LAUNCHER-LANG", "es")
            .header("X-NM-LAUNCHER-OS", "windows")
            .header("X-NM-LAUNCHER-OS-VERSION", "10.0.19045")
            .header("X-NM-LAUNCHER-ARCH", "x64")
            .header("X-NM-LAUNCHER-VER", NM_LAUNCHER_VER)
            .header("X-NM-LAUNCHER-SER-TY", "nm")
            .json(&req_body)
            .send()
            .await
            .context("Fallo en la petición HTTP a cpp-auth/token")?;

        let status = resp.status();
        let text = resp.text().await.context("Fallo al leer respuesta HTTP")?;

        if !status.is_success() {
            return Err(anyhow!(
                "Netmarble API devolvió HTTP {}: {}",
                status,
                text
            ));
        }

        let api_resp: NetmarbleApiResponse<String> = serde_json::from_str(&text)
            .with_context(|| format!("Respuesta no válida de Netmarble API: {}", text))?;

        if api_resp.code != 0 {
            return Err(anyhow!(
                "Error de Netmarble Auth (Código {}): {}",
                api_resp.code,
                api_resp.msg.unwrap_or_else(|| "Error desconocido".to_string())
            ));
        }

        let launcher_token = api_resp
            .data
            .ok_or_else(|| anyhow!("No se recibió launcherToken en la respuesta de Netmarble"))?;

        // Decodificar JWT para claims (expiración, player_id / sub)
        let (player_id, expires_at) = Self::decode_jwt_claims(&launcher_token);

        // Consultar el perfil del jugador
        let profile = Self::fetch_game_profile(&client, &launcher_token).await.ok();
        let profile_name = profile
            .as_ref()
            .and_then(|p| p.profile_name.clone())
            .unwrap_or_else(|| {
                if !player_id.is_empty() {
                    format!("Piloto #{}", &player_id[..player_id.len().min(6)])
                } else {
                    "Piloto Estelar".to_string()
                }
            });

        let profile_img_url = profile
            .as_ref()
            .and_then(|p| p.profile_img_url.clone())
            .unwrap_or_default();

        let session = AuthSession {
            launcher_token,
            channel: payload.channel.clone(),
            channel_code,
            player_id,
            profile_name,
            profile_img_url,
            expires_at,
            device_key: device_key.to_string(),
        };

        Self::save_session(&session)?;
        println!("✓ ¡Sesión oficial de Netmarble guardada con éxito para {}!", session.profile_name);

        Ok(session)
    }

    /// Permite procesar directamente un token de lanzador ya emitido (útil para cloud/fallback)
    pub async fn direct_token_login(launcher_token: &str) -> Result<AuthSession> {
        let client = reqwest::Client::builder().build()?;
        let (player_id, expires_at) = Self::decode_jwt_claims(launcher_token);
        let profile = Self::fetch_game_profile(&client, launcher_token).await.ok();

        let profile_name = profile
            .as_ref()
            .and_then(|p| p.profile_name.clone())
            .unwrap_or_else(|| {
                if !player_id.is_empty() {
                    format!("Piloto #{}", &player_id[..player_id.len().min(6)])
                } else {
                    "Piloto Estelar".to_string()
                }
            });

        let profile_img_url = profile
            .as_ref()
            .and_then(|p| p.profile_img_url.clone())
            .unwrap_or_default();

        let session = AuthSession {
            launcher_token: launcher_token.to_string(),
            channel: "manual".to_string(),
            channel_code: 0,
            player_id,
            profile_name,
            profile_img_url,
            expires_at,
            device_key: Self::get_or_create_device_key(),
        };

        Self::save_session(&session)?;
        Ok(session)
    }

    /// Consulta datos del perfil del jugador a Netmarble API
    async fn fetch_game_profile(
        client: &reqwest::Client,
        launcher_token: &str,
    ) -> Result<GameProfileData> {
        let url = format!("{}/game-profile?gameCode={}&platformAuthType=V5", NM_API_BASE, GAME_CODE);
        let resp = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", launcher_token))
            .header("Content-Type", "application/json")
            .header("X-NM-LAUNCHER-CH", NM_LAUNCHER_CH)
            .header("X-NM-LAUNCHER-VER", NM_LAUNCHER_VER)
            .send()
            .await?;

        let api_resp: NetmarbleApiResponse<GameProfileData> = resp.json().await?;
        if api_resp.code == 0 && api_resp.data.is_some() {
            Ok(api_resp.data.unwrap())
        } else {
            Err(anyhow!("No se pudo obtener el perfil de jugador"))
        }
    }

    /// Decodifica los claims del JWT de Netmarble sin verificar la firma (obtiene sub y exp)
    fn decode_jwt_claims(jwt: &str) -> (String, i64) {
        let parts: Vec<&str> = jwt.split('.').collect();
        if parts.len() < 2 {
            return (String::new(), 0);
        }

        use data_encoding::BASE64URL_NOPAD;
        let payload_b64 = parts[1];
        let decoded_bytes: Result<Vec<u8>, _> = BASE64URL_NOPAD.decode(payload_b64.as_bytes());

        if let Ok(bytes) = decoded_bytes {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                let sub = val["sub"].as_str().unwrap_or_default().to_string();
                let exp = val["exp"].as_i64().unwrap_or(0);
                return (sub, exp);
            }
        }

        (String::new(), 0)
    }

    /// Construye la URL oficial de SSO de Netmarble para abrir en el navegador
    pub fn build_auth_url(channel: &str, port: u16, lang: &str) -> String {
        let language = match lang {
            "es" | "es_ES" | "es-ES" => "es",
            "en" | "en_US" | "en-US" => "en",
            "ko" | "ko_KR" => "ko",
            "ja" => "ja",
            "zh" => "zh",
            _ => "es",
        };

        format!(
            "https://launcher.netmarble.com/v5/start?channel={}&gameCode={}&language={}&hostport={}",
            channel, GAME_CODE, language, port
        )
    }

    /// Inicia el servidor WebSocket en 127.0.0.1:port para esperar el callback del navegador
    pub async fn listen_for_auth_callback(
        port: u16,
    ) -> Result<(u16, tokio::sync::oneshot::Receiver<ChannelPayload>)> {
        let addr = format!("127.0.0.1:{}", port);
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(_) => {
                // Fallback a puerto aleatorio si el 55000 está ocupado
                let fallback = TcpListener::bind("127.0.0.1:0").await?;
                fallback
            }
        };

        let bound_port = listener.local_addr()?.port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx_mutex = Arc::new(Mutex::new(Some(tx)));

        tokio::spawn(async move {
            println!("Servidor de autenticación local escuchando en ws://127.0.0.1:{}/redirectLauncherV5", bound_port);

            // Timeout de 5 minutos para que el usuario complete el inicio de sesión
            let timeout_fut = tokio::time::sleep(std::time::Duration::from_secs(300));
            tokio::pin!(timeout_fut);

            loop {
                tokio::select! {
                    accept_res = listener.accept() => {
                        match accept_res {
                            Ok((stream, _peer)) => {
                                let tx_inner = tx_mutex.clone();
                                tokio::spawn(async move {
                                    match tokio_tungstenite::accept_async(stream).await {
                                        Ok(mut ws_stream) => {
                                            while let Some(msg_res) = ws_stream.next().await {
                                                if let Ok(msg) = msg_res {
                                                    if msg.is_text() {
                                                        let text = msg.to_text().unwrap_or_default();
                                                        println!("Mensaje recibido del navegador en WebSocket: {}", text);

                                                        if let Ok(payload) = serde_json::from_str::<ChannelPayload>(text) {
                                                            let mut opt = tx_inner.lock().await;
                                                            if let Some(sender) = opt.take() {
                                                                let _ = sender.send(payload);
                                                            }
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            eprintln!("Error en handshake WebSocket: {}", e);
                                        }
                                    }
                                });
                            }
                            Err(e) => {
                                eprintln!("Error al aceptar conexión TCP: {}", e);
                                break;
                            }
                        }
                    }
                    _ = &mut timeout_fut => {
                        println!("Timeout alcanzado en el servidor de autenticación local.");
                        break;
                    }
                }
            }
        });

        Ok((bound_port, rx))
    }
}
